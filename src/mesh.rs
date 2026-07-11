use rustc_hash::FxHashMap;
use std::io::{self, Read};

/// Binary wire format header magic: "MESH"
pub const MESH_MAGIC: u32 = 0x4D455348;
/// Wire format version. Bumped to 2 when per-vertex colors were added.
pub const MESH_VERSION: u32 = 2;

/// Fallback vertex color (linear-space) for meshes with no color info: matches the
/// previous hardcoded frontend material color `0x8ab4f8`, converted sRGB -> linear.
pub const DEFAULT_COLOR: [f32; 3] = [0.254_152_1, 0.456_411_02, 0.938_685_7];

/// Convert a single sRGB channel (0.0-1.0) to linear space.
pub fn srgb_channel_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Convert an sRGB color (0.0-1.0 per channel) to linear space.
pub fn srgb_to_linear(rgb: [f32; 3]) -> [f32; 3] {
    [
        srgb_channel_to_linear(rgb[0]),
        srgb_channel_to_linear(rgb[1]),
        srgb_channel_to_linear(rgb[2]),
    ]
}

/// A mesh in memory: indexed triangle mesh.
#[derive(Debug, Clone)]
pub struct Mesh {
    /// Flat array of vertex positions: [x0,y0,z0, x1,y1,z1, ...]
    pub positions: Vec<f32>,
    /// Flat array of triangle indices: [i0,i1,i2, ...]
    pub indices: Vec<u32>,
    /// Flat array of per-vertex linear-space colors: [r0,g0,b0, r1,g1,b1, ...]
    pub colors: Vec<f32>,
}

impl Mesh {
    pub fn new() -> Self {
        Self {
            positions: Vec::new(),
            indices: Vec::new(),
            colors: Vec::new(),
        }
    }

    pub fn vertex_count(&self) -> u32 {
        (self.positions.len() / 3) as u32
    }

    pub fn triangle_count(&self) -> u32 {
        (self.indices.len() / 3) as u32
    }

    /// Append another mesh into this one (with index offsetting).
    pub fn append(&mut self, other: &Mesh) {
        let offset = self.vertex_count();
        self.positions.extend_from_slice(&other.positions);
        self.indices
            .extend(other.indices.iter().map(|i| i + offset));
        self.colors.extend_from_slice(&other.colors);
    }

    /// Fill the color buffer with a single color repeated for every vertex.
    pub fn fill_color(&mut self, color: [f32; 3]) {
        self.colors.clear();
        self.colors.reserve(self.positions.len());
        for _ in 0..self.vertex_count() {
            self.colors.extend_from_slice(&color);
        }
    }

    /// Encode to the binary wire format.
    pub fn to_wire_format(&self) -> Vec<u8> {
        // The wire format is little-endian, and we bulk-copy the f32/u32 payload
        // with `bytemuck::cast_slice` (a native-endian reinterpret) instead of
        // serializing element-by-element — millions of `to_le_bytes` +
        // `extend_from_slice` calls were a measurable chunk of encode time. This
        // is only correct on little-endian targets; guard it at compile time.
        const _: () = assert!(
            cfg!(target_endian = "little"),
            "wire format assumes a little-endian target"
        );

        let n_verts = self.vertex_count();
        let n_tris = self.triangle_count();
        let header_size = 16; // 4 u32s
        let positions_size = self.positions.len() * 4;
        let indices_size = self.indices.len() * 4;
        let colors_size = self.colors.len() * 4;
        let total = header_size + positions_size + indices_size + colors_size;

        let mut buf = Vec::with_capacity(total);

        // Header
        buf.extend_from_slice(&MESH_MAGIC.to_le_bytes());
        buf.extend_from_slice(&MESH_VERSION.to_le_bytes());
        buf.extend_from_slice(&n_verts.to_le_bytes());
        buf.extend_from_slice(&n_tris.to_le_bytes());

        // Payload: bulk byte-copy the flat buffers (little-endian only, see above).
        buf.extend_from_slice(bytemuck::cast_slice(&self.positions));
        buf.extend_from_slice(bytemuck::cast_slice(&self.indices));
        buf.extend_from_slice(bytemuck::cast_slice(&self.colors));

        buf
    }
}

/// Key for vertex deduplication: quantize to 0.0001mm precision.
#[derive(Hash, Eq, PartialEq)]
struct VertexKey(i64, i64, i64);

fn quantize(v: f32) -> i64 {
    // Round to nearest 0.0001 mm
    (v * 10000.0).round() as i64
}

/// Read and parse a binary STL file, producing an indexed mesh with deduplicated vertices.
///
/// Binary STL format:
/// - 80 bytes: header (ignored)
/// - 4 bytes: u32 triangle count
/// - Per triangle (50 bytes each):
///   - 12 bytes: normal (f32 x3) — ignored, frontend recomputes
///   - 36 bytes: 3 vertices (f32 x3 each)
///   - 2 bytes: attribute byte count (ignored)
pub fn read_binary_stl<R: Read>(reader: &mut R) -> io::Result<Mesh> {
    // Read the whole input into memory once and parse from the slice. Binary STL
    // is dense, and this avoids a `read()` syscall per 50-byte triangle — the
    // dominant cost on large files (millions of triangles).
    let mut data = Vec::new();
    reader.read_to_end(&mut data)?;

    // Need at least the 80-byte header + 4-byte triangle count.
    if data.len() < 84 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "STL file too short",
        ));
    }

    let tri_count = u32::from_le_bytes([data[80], data[81], data[82], data[83]]);

    // Sanity check: prevent absurd allocations
    if tri_count > 50_000_000 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("STL triangle count too large: {}", tri_count),
        ));
    }

    // The buffer must actually hold every triangle we're about to index into.
    let expected_len = 84 + tri_count as usize * 50;
    if data.len() < expected_len {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "STL truncated: expected {} bytes for {} triangles, got {}",
                expected_len,
                tri_count,
                data.len()
            ),
        ));
    }

    // Pre-allocate with estimates
    let estimated_verts = (tri_count as usize).saturating_mul(3) / 2; // deduplicated estimate
    let mut mesh = Mesh {
        positions: Vec::with_capacity(estimated_verts * 3),
        indices: Vec::with_capacity(tri_count as usize * 3),
        colors: Vec::new(),
    };
    // FxHashMap is a fast non-cryptographic hasher. Vertex welding runs the hash
    // millions of times and doesn't need SipHash's DoS resistance.
    let mut vertex_map: FxHashMap<VertexKey, u32> =
        FxHashMap::with_capacity_and_hasher(estimated_verts, Default::default());

    for t in 0..tri_count as usize {
        let tri = &data[84 + t * 50..84 + t * 50 + 50];

        // Skip normal (first 12 bytes), read 3 vertices (bytes 12..48)
        for v in 0..3 {
            let base = 12 + v * 12;
            let x = f32::from_le_bytes([tri[base], tri[base + 1], tri[base + 2], tri[base + 3]]);
            let y =
                f32::from_le_bytes([tri[base + 4], tri[base + 5], tri[base + 6], tri[base + 7]]);
            let z =
                f32::from_le_bytes([tri[base + 8], tri[base + 9], tri[base + 10], tri[base + 11]]);

            let key = VertexKey(quantize(x), quantize(y), quantize(z));

            let idx = if let Some(&existing) = vertex_map.get(&key) {
                existing
            } else {
                let idx = mesh.vertex_count();
                mesh.positions.push(x);
                mesh.positions.push(y);
                mesh.positions.push(z);
                vertex_map.insert(key, idx);
                idx
            };

            mesh.indices.push(idx);
        }
        // 2 bytes attribute count at [48..50] — ignored
    }

    mesh.fill_color(DEFAULT_COLOR);
    Ok(mesh)
}

/// Read a binary STL from a file path.
pub fn read_stl_file(path: &std::path::Path) -> io::Result<Mesh> {
    let mut file = std::fs::File::open(path)?;
    read_binary_stl(&mut file)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a minimal binary STL with one triangle.
    fn make_test_stl() -> Vec<u8> {
        let mut data = Vec::new();

        // 80-byte header (zeros)
        data.extend_from_slice(&[0u8; 80]);

        // Triangle count: 1
        data.extend_from_slice(&1u32.to_le_bytes());

        // Normal (0,0,1)
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.extend_from_slice(&1.0f32.to_le_bytes());

        // Vertex 1: (0,0,0)
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());

        // Vertex 2: (1,0,0)
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());

        // Vertex 3: (0,1,0)
        data.extend_from_slice(&0.0f32.to_le_bytes());
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&0.0f32.to_le_bytes());

        // Attribute byte count
        data.extend_from_slice(&0u16.to_le_bytes());

        data
    }

    #[test]
    fn test_read_single_triangle_stl() {
        let data = make_test_stl();
        let mut cursor = std::io::Cursor::new(data);
        let mesh = read_binary_stl(&mut cursor).unwrap();

        assert_eq!(mesh.vertex_count(), 3);
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_vertex_deduplication() {
        // Two triangles sharing an edge: (0,0,0)-(1,0,0)-(0,1,0) and (1,0,0)-(1,1,0)-(0,1,0)
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 80]); // header
        data.extend_from_slice(&2u32.to_le_bytes()); // 2 triangles

        // Triangle 1
        data.extend_from_slice(&[0u8; 12]); // normal
        for v in [
            (0.0f32, 0.0f32, 0.0f32),
            (1.0f32, 0.0f32, 0.0f32),
            (0.0f32, 1.0f32, 0.0f32),
        ] {
            data.extend_from_slice(&v.0.to_le_bytes());
            data.extend_from_slice(&v.1.to_le_bytes());
            data.extend_from_slice(&v.2.to_le_bytes());
        }
        data.extend_from_slice(&0u16.to_le_bytes());

        // Triangle 2 — shares vertices (1,0,0) and (0,1,0) with triangle 1
        data.extend_from_slice(&[0u8; 12]); // normal
        for v in [
            (1.0f32, 0.0f32, 0.0f32),
            (1.0f32, 1.0f32, 0.0f32),
            (0.0f32, 1.0f32, 0.0f32),
        ] {
            data.extend_from_slice(&v.0.to_le_bytes());
            data.extend_from_slice(&v.1.to_le_bytes());
            data.extend_from_slice(&v.2.to_le_bytes());
        }
        data.extend_from_slice(&0u16.to_le_bytes());

        let mut cursor = std::io::Cursor::new(data);
        let mesh = read_binary_stl(&mut cursor).unwrap();

        // Should have 4 unique vertices (not 6)
        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn test_wire_format_roundtrip() {
        let data = make_test_stl();
        let mut cursor = std::io::Cursor::new(data);
        let mesh = read_binary_stl(&mut cursor).unwrap();
        let wire = mesh.to_wire_format();

        // Check header
        assert_eq!(
            u32::from_le_bytes([wire[0], wire[1], wire[2], wire[3]]),
            MESH_MAGIC
        );
        assert_eq!(
            u32::from_le_bytes([wire[4], wire[5], wire[6], wire[7]]),
            MESH_VERSION
        );
        assert_eq!(
            u32::from_le_bytes([wire[8], wire[9], wire[10], wire[11]]),
            3
        ); // nVerts
        assert_eq!(
            u32::from_le_bytes([wire[12], wire[13], wire[14], wire[15]]),
            1
        ); // nTris

        // Total size: 16 header + 3*3*4 positions + 1*3*4 indices + 3*3*4 colors = 100
        assert_eq!(wire.len(), 100);
    }

    #[test]
    fn test_stl_gets_default_color() {
        let data = make_test_stl();
        let mut cursor = std::io::Cursor::new(data);
        let mesh = read_binary_stl(&mut cursor).unwrap();

        assert_eq!(mesh.colors.len(), mesh.vertex_count() as usize * 3);
        for chunk in mesh.colors.chunks_exact(3) {
            assert_eq!(chunk, DEFAULT_COLOR);
        }
    }

    #[test]
    fn test_rejects_truncated_stl() {
        // Header claims 5 triangles but only one triangle's worth of data follows.
        let mut data = Vec::new();
        data.extend_from_slice(&[0u8; 80]); // header
        data.extend_from_slice(&5u32.to_le_bytes()); // claim 5 triangles
        data.extend_from_slice(&[0u8; 50]); // only 1 triangle present

        let mut cursor = std::io::Cursor::new(data);
        let result = read_binary_stl(&mut cursor);
        assert!(result.is_err());
    }

    #[test]
    fn test_srgb_to_linear_roundtrip_bounds() {
        assert!((srgb_channel_to_linear(0.0) - 0.0).abs() < 1e-6);
        assert!((srgb_channel_to_linear(1.0) - 1.0).abs() < 1e-6);
        // Midtones are darker in linear space than in sRGB.
        assert!(srgb_channel_to_linear(0.5) < 0.5);
    }
}
