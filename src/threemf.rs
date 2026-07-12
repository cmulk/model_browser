use crate::mesh::{self, Mesh};
use quick_xml::Reader;
use quick_xml::events::Event;
use std::collections::HashMap;
use std::io::{self, Read};
use std::path::Path;

/// A 4x3 affine transform stored row-major: m00 m01 m02 m10 m11 m12 m20 m21 m22 tx ty tz
/// Vertices transform as: v' = v·M + t (row-vector convention).
#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub m: [[f32; 3]; 3],
    pub t: [f32; 3],
}

impl Transform {
    /// Identity transform.
    pub fn identity() -> Self {
        Self {
            m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            t: [0.0, 0.0, 0.0],
        }
    }

    /// Parse from the 3MF "m00 m01 m02 m10 m11 m12 m20 m21 m22 tx ty tz" string.
    pub fn from_3mf_string(s: &str) -> Option<Self> {
        let parts: Vec<f32> = s
            .split_whitespace()
            .filter_map(|p| p.parse().ok())
            .collect();
        if parts.len() != 12 {
            return None;
        }
        Some(Self {
            m: [
                [parts[0], parts[1], parts[2]],
                [parts[3], parts[4], parts[5]],
                [parts[6], parts[7], parts[8]],
            ],
            t: [parts[9], parts[10], parts[11]],
        })
    }

    /// Compose two transforms: result = self * other.
    /// v' = ((v · other.M) + other.t) · self.M + self.t
    ///    = v · (other.M · self.M) + (other.t · self.M + self.t)
    pub fn compose(&self, other: &Transform) -> Transform {
        let mut result = Transform::identity();

        // M_result = other.M * self.M
        for i in 0..3 {
            for j in 0..3 {
                result.m[i][j] = other.m[i][0] * self.m[0][j]
                    + other.m[i][1] * self.m[1][j]
                    + other.m[i][2] * self.m[2][j];
            }
        }

        // t_result = other.t * self.M + self.t
        for j in 0..3 {
            result.t[j] = other.t[0] * self.m[0][j]
                + other.t[1] * self.m[1][j]
                + other.t[2] * self.m[2][j]
                + self.t[j];
        }

        result
    }

    /// Apply this transform to a vertex (row-vector): v' = v·M + t.
    pub fn apply(&self, x: f32, y: f32, z: f32) -> (f32, f32, f32) {
        (
            x * self.m[0][0] + y * self.m[1][0] + z * self.m[2][0] + self.t[0],
            x * self.m[0][1] + y * self.m[1][1] + z * self.m[2][1] + self.t[1],
            x * self.m[0][2] + y * self.m[1][2] + z * self.m[2][2] + self.t[2],
        )
    }
}

/// A component reference within a 3MF object.
#[derive(Debug, Clone)]
struct ComponentRef {
    /// External file path (from p:path attribute), None = same file
    path: Option<String>,
    /// Object ID within the target file
    object_id: u32,
    /// Transform to apply
    transform: Transform,
}

/// A parsed 3MF object — either has inline mesh data or is made of components.
#[derive(Debug, Clone)]
enum ObjectData {
    MeshData(Mesh),
    Components(Vec<ComponentRef>),
}

/// A build item from the <build> section.
#[derive(Debug, Clone)]
struct BuildItem {
    object_id: u32,
    transform: Transform,
}

/// Parse an object model file (either root 3dmodel.model or an external object_N.model).
/// Returns a map of object_id -> ObjectData.
fn parse_model_xml(xml_data: &[u8]) -> io::Result<HashMap<u32, ObjectData>> {
    let mut reader = Reader::from_reader(xml_data);
    reader.config_mut().trim_text(true);

    let mut objects: HashMap<u32, ObjectData> = HashMap::new();
    let mut buf = Vec::with_capacity(4096);

    // State tracking
    let mut current_object_id: Option<u32> = None;
    let mut in_mesh = false;
    let mut in_vertices = false;
    let mut in_triangles = false;
    let mut current_vertices: Vec<f32> = Vec::new();
    let mut current_indices: Vec<u32> = Vec::new();
    let mut current_components: Vec<ComponentRef> = Vec::new();
    let mut has_mesh = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let local_name = e.local_name();
                let name = local_name.as_ref();
                match name {
                    b"object" => {
                        // Extract id attribute
                        if let Some(id) = get_attr(e, b"id") {
                            current_object_id = id.parse().ok();
                            current_vertices.clear();
                            current_indices.clear();
                            current_components.clear();
                            has_mesh = false;
                            in_mesh = false;
                        }
                    }
                    b"mesh" => {
                        if current_object_id.is_some() {
                            in_mesh = true;
                            has_mesh = true;
                        }
                    }
                    b"vertices" => {
                        if in_mesh {
                            in_vertices = true;
                        }
                    }
                    b"vertex" => {
                        if in_vertices {
                            current_vertices.extend_from_slice(&parse_vertex(e));
                        }
                    }
                    b"triangles" => {
                        if in_mesh {
                            in_triangles = true;
                        }
                    }
                    b"triangle" => {
                        if in_triangles {
                            current_indices.extend_from_slice(&parse_triangle(e));
                        }
                    }
                    b"component" if current_object_id.is_some() => {
                        let object_id = get_attr(e, b"objectid")
                            .and_then(|v| v.parse::<u32>().ok())
                            .unwrap_or(0);

                        // Try p:path attribute (production extension)
                        let path = get_attr_any_ns(e, b"path");

                        let transform = get_attr(e, b"transform")
                            .and_then(|s| Transform::from_3mf_string(&s))
                            .unwrap_or_else(Transform::identity);

                        current_components.push(ComponentRef {
                            path,
                            object_id,
                            transform,
                        });
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => {
                let local_name = e.local_name();
                let name = local_name.as_ref();
                match name {
                    b"object" => {
                        if let Some(obj_id) = current_object_id.take() {
                            if has_mesh {
                                objects.insert(
                                    obj_id,
                                    ObjectData::MeshData(Mesh {
                                        positions: std::mem::take(&mut current_vertices),
                                        indices: std::mem::take(&mut current_indices),
                                        colors: Vec::new(),
                                    }),
                                );
                            } else if !current_components.is_empty() {
                                objects.insert(
                                    obj_id,
                                    ObjectData::Components(std::mem::take(&mut current_components)),
                                );
                            }
                        }
                        in_mesh = false;
                        in_vertices = false;
                        in_triangles = false;
                    }
                    b"mesh" => {
                        in_mesh = false;
                    }
                    b"vertices" => {
                        in_vertices = false;
                    }
                    b"triangles" => {
                        in_triangles = false;
                    }
                    _ => {}
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("XML parse error: {}", e),
                ));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(objects)
}

/// Parse just the <build> section from the root model XML.
fn parse_build_items(xml_data: &[u8]) -> io::Result<Vec<BuildItem>> {
    let mut reader = Reader::from_reader(xml_data);
    reader.config_mut().trim_text(true);

    let mut buf = Vec::with_capacity(4096);
    let mut items = Vec::new();
    let mut in_build = false;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) => {
                let local_name = e.local_name();
                if local_name.as_ref() == b"build" {
                    in_build = true;
                }
            }
            Ok(Event::Empty(ref e)) => {
                let local_name = e.local_name();
                if in_build && local_name.as_ref() == b"item" {
                    let object_id = get_attr(e, b"objectid")
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(0);
                    let transform = get_attr(e, b"transform")
                        .and_then(|s| Transform::from_3mf_string(&s))
                        .unwrap_or_else(Transform::identity);
                    items.push(BuildItem {
                        object_id,
                        transform,
                    });
                }
            }
            Ok(Event::End(ref e)) => {
                let local_name = e.local_name();
                if local_name.as_ref() == b"build" {
                    in_build = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("XML parse error: {}", e),
                ));
            }
            _ => {}
        }
        buf.clear();
    }

    Ok(items)
}

/// Per-object color assignment info parsed from Bambu Studio's `model_settings.config`:
/// an object-level default extruder plus per-part (component) overrides.
#[derive(Debug, Clone, Default)]
struct ObjectColorMeta {
    default_extruder: Option<u32>,
    part_extruders: HashMap<u32, u32>,
}

/// Parse a `#RRGGBB` or `#RRGGBBAA` hex color string into a linear-space RGB triple.
fn parse_hex_color(s: &str) -> Option<[f32; 3]> {
    let hex = s.strip_prefix('#')?;
    if hex.len() < 6 {
        return None;
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    Some(mesh::srgb_to_linear([
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
    ]))
}

/// Parse the `filament_colour` array out of a Bambu/Prusa `project_settings.config` JSON blob.
/// Index `i` corresponds to extruder number `i + 1`. Best-effort: returns an empty Vec if the
/// file is missing, malformed, or has no color array.
fn parse_filament_colors(json_data: &[u8]) -> Vec<[f32; 3]> {
    let value: serde_json::Value = match serde_json::from_slice(json_data) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let Some(colors) = value.get("filament_colour").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    colors
        .iter()
        .map(|c| {
            c.as_str()
                .and_then(parse_hex_color)
                .unwrap_or(mesh::DEFAULT_COLOR)
        })
        .collect()
}

/// Parse Bambu Studio's `model_settings.config` XML into a map of object id -> color metadata.
/// Best-effort: returns an empty map on any structural surprise rather than failing the load,
/// since color assignment is a rendering enhancement, not required to display geometry.
fn parse_object_color_meta(xml_data: &[u8]) -> HashMap<u32, ObjectColorMeta> {
    let mut reader = Reader::from_reader(xml_data);
    reader.config_mut().trim_text(true);

    let mut result: HashMap<u32, ObjectColorMeta> = HashMap::new();
    let mut buf = Vec::with_capacity(4096);

    let mut current_object_id: Option<u32> = None;
    let mut current_part_id: Option<u32> = None;

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                let name = e.local_name();
                match name.as_ref() {
                    b"object" => {
                        current_object_id = get_attr(e, b"id").and_then(|v| v.parse().ok());
                        current_part_id = None;
                        if let Some(id) = current_object_id {
                            result.entry(id).or_default();
                        }
                    }
                    b"part" => {
                        current_part_id = get_attr(e, b"id").and_then(|v| v.parse().ok());
                    }
                    b"metadata" => {
                        if get_attr(e, b"key").as_deref() != Some("extruder") {
                            continue;
                        }
                        let Some(extruder) =
                            get_attr(e, b"value").and_then(|v| v.parse::<u32>().ok())
                        else {
                            continue;
                        };
                        let Some(obj_id) = current_object_id else {
                            continue;
                        };
                        let entry = result.entry(obj_id).or_default();
                        match current_part_id {
                            Some(part_id) => {
                                entry.part_extruders.insert(part_id, extruder);
                            }
                            None => entry.default_extruder = Some(extruder),
                        }
                    }
                    _ => {}
                }
            }
            Ok(Event::End(ref e)) => match e.local_name().as_ref() {
                b"object" => {
                    current_object_id = None;
                    current_part_id = None;
                }
                b"part" => current_part_id = None,
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }

    result
}

/// Resolve an extruder number to a linear-space color, falling back when there's no
/// override or the extruder index is out of range.
fn extruder_color(
    filament_colors: &[[f32; 3]],
    extruder: Option<u32>,
    fallback: [f32; 3],
) -> [f32; 3] {
    extruder
        .and_then(|e| filament_colors.get((e as usize).saturating_sub(1)))
        .copied()
        .unwrap_or(fallback)
}

/// Get an attribute value by local name (ignoring namespace).
fn get_attr(e: &quick_xml::events::BytesStart<'_>, attr_name: &[u8]) -> Option<String> {
    for attr in e.attributes().flatten() {
        let key = attr.key.local_name();
        if key.as_ref() == attr_name {
            return String::from_utf8(attr.value.to_vec()).ok();
        }
    }
    None
}

/// Get attribute by local name, checking any namespace prefix (for p:path).
fn get_attr_any_ns(
    e: &quick_xml::events::BytesStart<'_>,
    attr_local_name: &[u8],
) -> Option<String> {
    for attr in e.attributes().flatten() {
        let key_bytes = attr.key.as_ref();
        // Match "path" or "p:path" or any "ns:path"
        let local = if let Some(pos) = key_bytes.iter().position(|&b| b == b':') {
            &key_bytes[pos + 1..]
        } else {
            key_bytes
        };
        if local == attr_local_name {
            return String::from_utf8(attr.value.to_vec()).ok();
        }
    }
    None
}

/// Parse the x/y/z coordinates of a `<vertex>` in a single pass over its attributes,
/// reading numbers directly from the borrowed bytes. This is the hot path (called once
/// per vertex, millions of times on large models); the old `get_attr` allocated a
/// `String` per lookup and re-scanned the attribute list for each of x, y, z.
fn parse_vertex(e: &quick_xml::events::BytesStart<'_>) -> [f32; 3] {
    let mut v = [0.0f32; 3];
    for attr in e.attributes().flatten() {
        let slot = match attr.key.local_name().as_ref() {
            b"x" => &mut v[0],
            b"y" => &mut v[1],
            b"z" => &mut v[2],
            _ => continue,
        };
        if let Ok(f) = std::str::from_utf8(&attr.value)
            .unwrap_or("")
            .parse::<f32>()
        {
            *slot = f;
        }
    }
    v
}

/// Parse the v1/v2/v3 indices of a `<triangle>` in a single pass, allocation-free.
/// Hot path — see [`parse_vertex`].
fn parse_triangle(e: &quick_xml::events::BytesStart<'_>) -> [u32; 3] {
    let mut t = [0u32; 3];
    for attr in e.attributes().flatten() {
        let slot = match attr.key.local_name().as_ref() {
            b"v1" => &mut t[0],
            b"v2" => &mut t[1],
            b"v3" => &mut t[2],
            _ => continue,
        };
        if let Ok(n) = std::str::from_utf8(&attr.value)
            .unwrap_or("")
            .parse::<u32>()
        {
            *slot = n;
        }
    }
    t
}

/// Read and parse a 3MF file into a unified mesh.
///
/// Handles the Bambu Studio production extension where geometry lives in
/// external `3D/Objects/object_N.model` files referenced via `p:path` attributes.
pub fn read_3mf_file(path: &Path) -> io::Result<Mesh> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Not a valid zip/3mf: {}", e),
        )
    })?;

    // Read root model file
    let root_xml = read_zip_entry(&mut archive, "3D/3dmodel.model")?;

    // Parse objects and build items from root
    let root_objects = parse_model_xml(&root_xml)?;
    let build_items = parse_build_items(&root_xml)?;

    if build_items.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "No build items found in 3MF file",
        ));
    }

    // Best-effort color metadata (Bambu Studio-specific, may be absent for other slicers).
    let filament_colors = read_zip_entry(&mut archive, "Metadata/project_settings.config")
        .map(|data| parse_filament_colors(&data))
        .unwrap_or_default();
    let object_color_meta = read_zip_entry(&mut archive, "Metadata/model_settings.config")
        .map(|data| parse_object_color_meta(&data))
        .unwrap_or_default();

    // Cache for parsed external model files: path -> (object_id -> ObjectData)
    let mut external_cache: HashMap<String, HashMap<u32, ObjectData>> = HashMap::new();
    external_cache.insert("".to_string(), root_objects);

    let mut combined_mesh = Mesh::new();

    // Resolve each build item
    for item in &build_items {
        let default_extruder = object_color_meta
            .get(&item.object_id)
            .and_then(|m| m.default_extruder);
        let root_color = extruder_color(&filament_colors, default_extruder, mesh::DEFAULT_COLOR);

        resolve_object(
            item.object_id,
            &item.transform,
            "",
            &mut archive,
            &mut external_cache,
            &mut combined_mesh,
            0, // recursion depth
            item.object_id,
            root_color,
            &object_color_meta,
            &filament_colors,
        )?;
    }

    Ok(combined_mesh)
}

/// Recursively resolve an object, applying accumulated transforms and colors.
///
/// `root_object_id` stays fixed at the top-level build item's object id for the whole
/// recursion, since Bambu Studio's `model_settings.config` keys per-part color overrides
/// by that top-level id (see `ObjectColorMeta`). `color` is the color inherited from the
/// parent, overridden per-component when a part-level extruder override is found.
#[allow(clippy::too_many_arguments)]
fn resolve_object(
    object_id: u32,
    parent_transform: &Transform,
    current_file_key: &str,
    archive: &mut zip::ZipArchive<std::fs::File>,
    external_cache: &mut HashMap<String, HashMap<u32, ObjectData>>,
    combined_mesh: &mut Mesh,
    depth: u32,
    root_object_id: u32,
    color: [f32; 3],
    object_color_meta: &HashMap<u32, ObjectColorMeta>,
    filament_colors: &[[f32; 3]],
) -> io::Result<()> {
    if depth > 20 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "3MF component recursion too deep",
        ));
    }

    // Step 1: Check if it is a Mesh
    let mut mesh_to_append = None;
    if let Some(ObjectData::MeshData(mesh)) = external_cache
        .get(current_file_key)
        .and_then(|objects| objects.get(&object_id))
    {
        let mut transformed = Mesh::new();
        transformed.positions.reserve(mesh.positions.len());
        transformed.indices.reserve(mesh.indices.len());

        for chunk in mesh.positions.chunks_exact(3) {
            let (x, y, z) = parent_transform.apply(chunk[0], chunk[1], chunk[2]);
            transformed.positions.push(x);
            transformed.positions.push(y);
            transformed.positions.push(z);
        }
        transformed.indices = mesh.indices.clone();
        transformed.fill_color(color);
        mesh_to_append = Some(transformed);
    }

    if let Some(transformed) = mesh_to_append {
        combined_mesh.append(&transformed);
        return Ok(());
    }

    // Step 2: Check if it is Components
    let mut components_to_process = None;
    if let Some(ObjectData::Components(components)) = external_cache
        .get(current_file_key)
        .and_then(|objects| objects.get(&object_id))
    {
        components_to_process = Some(components.clone());
    }

    if let Some(components) = components_to_process {
        for comp in components {
            let child_transform = parent_transform.compose(&comp.transform);

            let part_extruder = object_color_meta
                .get(&root_object_id)
                .and_then(|m| m.part_extruders.get(&comp.object_id))
                .copied();
            let child_color = extruder_color(filament_colors, part_extruder, color);

            if let Some(ref ext_path) = comp.path {
                // External file reference (production extension)
                let normalized_path = normalize_zip_path(ext_path);

                // Get or parse the external file
                if !external_cache.contains_key(&normalized_path) {
                    let ext_xml = read_zip_entry(archive, &normalized_path)?;
                    let ext_objects = parse_model_xml(&ext_xml)?;
                    external_cache.insert(normalized_path.clone(), ext_objects);
                }

                resolve_object(
                    comp.object_id,
                    &child_transform,
                    &normalized_path,
                    archive,
                    external_cache,
                    combined_mesh,
                    depth + 1,
                    root_object_id,
                    child_color,
                    object_color_meta,
                    filament_colors,
                )?;
            } else {
                // Same file reference
                resolve_object(
                    comp.object_id,
                    &child_transform,
                    current_file_key,
                    archive,
                    external_cache,
                    combined_mesh,
                    depth + 1,
                    root_object_id,
                    child_color,
                    object_color_meta,
                    filament_colors,
                )?;
            }
        }
        return Ok(());
    }

    // Object ID not found in current file key
    tracing::warn!(
        "Object {} not found in file '{}'",
        object_id,
        current_file_key
    );
    Ok(())
}

/// Normalize a zip entry path: strip leading '/' and convert backslashes.
fn normalize_zip_path(path: &str) -> String {
    let p = path.replace('\\', "/");
    let p = p.trim_start_matches('/');
    // Security: reject paths with .. components
    if p.split('/').any(|c| c == "..") {
        tracing::warn!("Rejected zip path with traversal: {}", path);
        return String::new();
    }
    p.to_string()
}

/// Read a zip entry by name, returning its full contents.
fn read_zip_entry(archive: &mut zip::ZipArchive<std::fs::File>, name: &str) -> io::Result<Vec<u8>> {
    let mut entry = archive.by_name(name).map_err(|e| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("Zip entry '{}' not found: {}", name, e),
        )
    })?;

    let mut data = Vec::with_capacity(entry.size() as usize);
    entry.read_to_end(&mut data)?;
    Ok(data)
}

/// Serialize a mesh into a minimal single-object 3MF package (zip bytes).
///
/// Used to hand STL files to Bambu Studio: its `bambustudio://open?file=` handler
/// only accepts `.3mf` downloads, so STLs are converted on the fly.
pub fn write_3mf(mesh: &Mesh) -> io::Result<Vec<u8>> {
    use std::fmt::Write as _;
    use std::io::Write as _;

    const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
 <Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
 <Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/>
</Types>"#;

    const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
 <Relationship Target="/3D/3dmodel.model" Id="rel-1" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/>
</Relationships>"#;

    // ~40 bytes per vertex/triangle line is a decent estimate; avoids repeated growth.
    let mut model = String::with_capacity(
        512 + (mesh.positions.len() / 3) * 40 + (mesh.indices.len() / 3) * 40,
    );
    model.push_str(concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<model unit=\"millimeter\" xml:lang=\"en-US\" ",
        "xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n",
        "<resources>\n<object id=\"1\" type=\"model\">\n<mesh>\n<vertices>\n"
    ));
    for v in mesh.positions.chunks_exact(3) {
        let _ = writeln!(
            model,
            "<vertex x=\"{}\" y=\"{}\" z=\"{}\"/>",
            v[0], v[1], v[2]
        );
    }
    model.push_str("</vertices>\n<triangles>\n");
    for t in mesh.indices.chunks_exact(3) {
        let _ = writeln!(
            model,
            "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>",
            t[0], t[1], t[2]
        );
    }
    model.push_str(concat!(
        "</triangles>\n</mesh>\n</object>\n</resources>\n",
        "<build>\n<item objectid=\"1\"/>\n</build>\n</model>\n"
    ));

    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    for (name, data) in [
        ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
        ("_rels/.rels", RELS.as_bytes()),
        ("3D/3dmodel.model", model.as_bytes()),
    ] {
        writer.start_file(name, opts).map_err(io::Error::other)?;
        writer.write_all(data)?;
    }

    let cursor = writer.finish().map_err(io::Error::other)?;
    Ok(cursor.into_inner())
}

/// Extract the thumbnail PNG from a .3mf file.
///
/// Tries in order: Metadata/plate_1.png, Metadata/plate_1_small.png, Metadata/top_1.png
pub fn extract_thumbnail(path: &Path) -> io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("Not a valid zip/3mf: {}", e),
        )
    })?;

    let candidates = [
        "Metadata/plate_1.png",
        "Metadata/plate_1_small.png",
        "Metadata/top_1.png",
    ];

    for name in &candidates {
        if let Ok(data) = read_zip_entry(&mut archive, name) {
            return Ok(data);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "No thumbnail found in 3MF file",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_transform() {
        let t = Transform::identity();
        let (x, y, z) = t.apply(1.0, 2.0, 3.0);
        assert!((x - 1.0).abs() < 1e-6);
        assert!((y - 2.0).abs() < 1e-6);
        assert!((z - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_translation_transform() {
        let t = Transform {
            m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            t: [10.0, 20.0, 30.0],
        };
        let (x, y, z) = t.apply(1.0, 2.0, 3.0);
        assert!((x - 11.0).abs() < 1e-6);
        assert!((y - 22.0).abs() < 1e-6);
        assert!((z - 33.0).abs() < 1e-6);
    }

    #[test]
    fn test_parse_3mf_transform_string() {
        let t = Transform::from_3mf_string("1 0 0 0 1 0 0 0 1 2.977 -21.596 0.346").unwrap();
        assert!((t.m[0][0] - 1.0).abs() < 1e-6);
        assert!((t.t[0] - 2.977).abs() < 1e-3);
        assert!((t.t[1] - (-21.596)).abs() < 1e-3);
    }

    #[test]
    fn test_compose_translations() {
        let parent = Transform {
            m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            t: [10.0, 0.0, 0.0],
        };
        let child = ComponentRef {
            path: None,
            object_id: 1,
            transform: Transform {
                m: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                t: [5.0, 0.0, 0.0],
            },
        };
        let result = parent.compose(&child.transform);
        // Applying (0,0,0) should give (15,0,0) — both translations combined
        let (x, _y, _z) = result.apply(0.0, 0.0, 0.0);
        assert!((x - 15.0).abs() < 1e-6);
    }

    #[test]
    fn test_normalize_zip_path() {
        assert_eq!(
            normalize_zip_path("/3D/Objects/object_15.model"),
            "3D/Objects/object_15.model"
        );
        assert_eq!(
            normalize_zip_path("3D\\Objects\\object_15.model"),
            "3D/Objects/object_15.model"
        );
        // Traversal should be rejected
        assert_eq!(normalize_zip_path("../../../etc/passwd"), "");
    }

    #[test]
    fn test_parse_model_xml_mesh() {
        // A minimal single-object mesh: one triangle, exercising parse_vertex/parse_triangle
        // (including a missing coordinate that should default to 0 and scientific notation).
        let xml = br#"<model>
  <resources>
    <object id="7" type="model">
      <mesh>
        <vertices>
          <vertex x="0" y="0" z="0"/>
          <vertex x="1.5" z="3e2"/>
          <vertex x="0" y="1" z="0"/>
        </vertices>
        <triangles>
          <triangle v1="0" v2="1" v3="2"/>
        </triangles>
      </mesh>
    </object>
  </resources>
</model>"#;

        let objects = parse_model_xml(xml).unwrap();
        let ObjectData::MeshData(mesh) = objects.get(&7).expect("object 7") else {
            panic!("expected mesh data");
        };
        assert_eq!(mesh.positions.len(), 9); // 3 vertices * 3 coords
        assert_eq!(mesh.indices, vec![0, 1, 2]);
        // Vertex 1: x parsed, y missing -> 0, z scientific notation.
        assert!((mesh.positions[3] - 1.5).abs() < 1e-6);
        assert_eq!(mesh.positions[4], 0.0);
        assert!((mesh.positions[5] - 300.0).abs() < 1e-3);
    }

    #[test]
    fn test_write_3mf_roundtrip() {
        // One triangle in, write as 3MF, read back through the full 3MF parser.
        let mut src = Mesh {
            positions: vec![0.0, 0.0, 0.0, 10.5, 0.0, 0.0, 0.0, 7.25, 0.0],
            indices: vec![0, 1, 2],
            colors: Vec::new(),
        };
        src.fill_color(mesh::DEFAULT_COLOR);

        let bytes = write_3mf(&src).unwrap();

        let dir = std::env::temp_dir().join("model_browser_test_write3mf");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("roundtrip.3mf");
        std::fs::write(&path, &bytes).unwrap();
        let parsed = read_3mf_file(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(parsed.vertex_count(), 3);
        assert_eq!(parsed.triangle_count(), 1);
        assert_eq!(parsed.indices, src.indices);
        for (a, b) in parsed.positions.iter().zip(src.positions.iter()) {
            assert!((a - b).abs() < 1e-6);
        }
    }

    #[test]
    fn test_parse_hex_color() {
        let black = parse_hex_color("#000000").unwrap();
        assert!(black.iter().all(|&c| c.abs() < 1e-6));

        let white = parse_hex_color("#FFFFFF").unwrap();
        assert!(white.iter().all(|&c| (c - 1.0).abs() < 1e-6));

        // 8-digit hex (with alpha) should still parse the RGB portion
        assert!(parse_hex_color("#FF000080").is_some());

        assert!(parse_hex_color("not-a-color").is_none());
        assert!(parse_hex_color("#FFF").is_none());
    }

    #[test]
    fn test_parse_filament_colors() {
        let json = br##"{"filament_colour": ["#000000", "#FFFFFF", "#D2B79B"]}"##;
        let colors = parse_filament_colors(json);
        assert_eq!(colors.len(), 3);
        assert!(colors[0].iter().all(|&c| c.abs() < 1e-6));
        assert!(colors[1].iter().all(|&c| (c - 1.0).abs() < 1e-6));
    }

    #[test]
    fn test_parse_filament_colors_missing_field() {
        let json = br#"{"other_key": "value"}"#;
        assert!(parse_filament_colors(json).is_empty());
    }

    #[test]
    fn test_parse_object_color_meta() {
        let xml = br#"<?xml version="1.0"?>
<config>
  <object id="12">
    <metadata key="extruder" value="4"/>
    <part id="1" subtype="normal_part">
      <metadata key="extruder" value="1"/>
    </part>
    <part id="2" subtype="normal_part">
      <metadata key="matrix" value="1 0 0 0"/>
    </part>
  </object>
</config>"#;
        let meta = parse_object_color_meta(xml);
        let obj = meta.get(&12).unwrap();
        assert_eq!(obj.default_extruder, Some(4));
        assert_eq!(obj.part_extruders.get(&1), Some(&1));
        // Part 2 has no extruder override, so it should be absent (falls back to default).
        assert_eq!(obj.part_extruders.get(&2), None);
    }

    #[test]
    fn test_extruder_color_falls_back() {
        let colors = vec![[0.0, 0.0, 0.0], [1.0, 1.0, 1.0]];
        // Extruder 1 -> index 0
        assert_eq!(
            extruder_color(&colors, Some(1), mesh::DEFAULT_COLOR),
            [0.0, 0.0, 0.0]
        );
        // Out-of-range extruder falls back
        assert_eq!(
            extruder_color(&colors, Some(99), mesh::DEFAULT_COLOR),
            mesh::DEFAULT_COLOR
        );
        // No extruder falls back
        assert_eq!(
            extruder_color(&colors, None, mesh::DEFAULT_COLOR),
            mesh::DEFAULT_COLOR
        );
    }
}
