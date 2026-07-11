import * as THREE from 'three';
import { OrbitControls } from 'three/addons/controls/OrbitControls.js';

// ─── State ─────────────────────────────────────────────────────────────────────

let scene, camera, renderer, controls, currentMesh;
let selectedFileEl = null;
let currentFilePath = null;
let showingThumbnail = false;
let allFileElements = [];
let allDirElements = [];
let visibleFileIndex = -1;

// ─── DOM References ────────────────────────────────────────────────────────────

const treeScroll = document.getElementById('tree-scroll');
const treeLoading = document.getElementById('tree-loading');
const searchInput = document.getElementById('search-input');
const previewEmpty = document.getElementById('preview-empty');
const previewContent = document.getElementById('preview-content');
const previewViewport = document.getElementById('preview-viewport');
const previewCanvas = document.getElementById('preview-canvas');
const previewImage = document.getElementById('preview-image');
const previewThumbnail = document.getElementById('preview-thumbnail');
const previewProgress = document.getElementById('preview-progress');
const progressBar = document.getElementById('progress-bar');
const progressText = document.getElementById('progress-text');
const infoFilename = document.getElementById('info-filename');
const infoSize = document.getElementById('info-size');
const infoStats = document.getElementById('info-stats');
const btnDownload = document.getElementById('btn-download');
const btnThumbnail = document.getElementById('btn-thumbnail');
const btnReset = document.getElementById('btn-reset');
const splitter = document.getElementById('splitter');
const treePanel = document.getElementById('tree-panel');
const toastContainer = document.getElementById('toast-container');

// ─── Three.js Setup ────────────────────────────────────────────────────────────

function initThree() {
    scene = new THREE.Scene();

    camera = new THREE.PerspectiveCamera(45, 1, 0.1, 10000);
    camera.up.set(0, 0, 1); // Z-up for 3MF/STL
    camera.position.set(100, -150, 80);

    renderer = new THREE.WebGLRenderer({
        canvas: previewCanvas,
        antialias: true,
        alpha: true,
    });
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.setClearColor(0x000000, 0);
    renderer.outputColorSpace = THREE.SRGBColorSpace;
    renderer.toneMapping = THREE.ACESFilmicToneMapping;
    renderer.toneMappingExposure = 1.2;

    // Lighting
    const hemiLight = new THREE.HemisphereLight(0xb0c4ff, 0x2a1a3a, 0.8);
    scene.add(hemiLight);

    const keyLight = new THREE.DirectionalLight(0xffffff, 1.2);
    keyLight.position.set(80, -60, 120);
    scene.add(keyLight);

    const fillLight = new THREE.DirectionalLight(0x8090c0, 0.4);
    fillLight.position.set(-60, 80, 40);
    scene.add(fillLight);

    const rimLight = new THREE.DirectionalLight(0xa0a0ff, 0.3);
    rimLight.position.set(0, 60, -40);
    scene.add(rimLight);

    // Controls
    controls = new OrbitControls(camera, renderer.domElement);
    controls.enableDamping = true;
    controls.dampingFactor = 0.08;
    controls.rotateSpeed = 0.8;
    controls.zoomSpeed = 1.2;
    controls.minDistance = 1;
    controls.maxDistance = 5000;

    // Subtle grid
    const grid = new THREE.GridHelper(400, 40, 0x2a2a4a, 0x1a1a3a);
    grid.rotation.x = Math.PI / 2; // Align to XY plane (Z-up)
    grid.position.z = -0.1;
    grid.material.opacity = 0.3;
    grid.material.transparent = true;
    scene.add(grid);

    resizeRenderer();
    animate();
}

function resizeRenderer() {
    const rect = previewViewport.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;
    if (w > 0 && h > 0) {
        renderer.setSize(w, h);
        camera.aspect = w / h;
        camera.updateProjectionMatrix();
    }
}

function animate() {
    requestAnimationFrame(animate);
    controls.update();
    renderer.render(scene, camera);
}

// ─── SVG Icon Helpers (using DOMParser for security) ───────────────────────────

function createSVG(svgString) {
    const doc = new DOMParser().parseFromString(svgString, 'image/svg+xml');
    return doc.documentElement;
}

function chevronSVG() {
    return '<svg viewBox="0 0 16 16" fill="currentColor"><path d="M6 3l5 5-5 5V3z"/></svg>';
}

function cubeSVG() {
    return '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><path d="M8 1.5L14 5v6l-6 3.5L2 11V5L8 1.5z"/><path d="M8 8.5V15M8 8.5L14 5M8 8.5L2 5"/></svg>';
}

function wireframeSVG() {
    return '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1"><path d="M8 1.5L14 5v6l-6 3.5L2 11V5L8 1.5z"/><path d="M8 8.5V15M8 8.5L14 5M8 8.5L2 5" stroke-dasharray="2 1"/></svg>';
}

function imageSVG() {
    return '<svg viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.2"><rect x="2" y="2" width="12" height="12" rx="1.5"/><circle cx="5.5" cy="5.5" r="1.5"/><path d="M2 11l3-3 2 2 3-3 4 4"/></svg>';
}

// ─── Tree Building ─────────────────────────────────────────────────────────────

async function loadTree() {
    try {
        const resp = await fetch('/api/tree');
        if (!resp.ok) throw new Error('Failed to load tree');
        const data = await resp.json();
        renderTree(data);
    } catch (e) {
        treeLoading.textContent = 'Failed to load library';
        showToast('Failed to load model library', true);
    }
}

function renderTree(rootNode) {
    treeLoading.style.display = 'none';
    allFileElements = [];
    allDirElements = [];

    // Clear tree
    const existing = treeScroll.querySelectorAll('.tree-dir, .tree-file');
    existing.forEach(el => el.remove());

    // Render dirs
    for (const dir of rootNode.dirs) {
        treeScroll.appendChild(buildDirNode(dir, 0));
    }
    // Render root-level files
    for (const file of rootNode.files) {
        treeScroll.appendChild(buildFileNode(file));
    }
}

function buildDirNode(dir, depth) {
    const container = document.createElement('div');
    container.className = 'tree-dir';

    const header = document.createElement('div');
    header.className = 'tree-dir-header';
    header.style.paddingLeft = (8 + depth * 12) + 'px';

    // Chevron
    const chevron = document.createElement('span');
    chevron.className = 'tree-chevron';
    const chevSvg = createSVG(chevronSVG());
    chevron.appendChild(chevSvg);

    // Folder icon
    const icon = document.createElement('span');
    icon.className = 'tree-dir-icon';
    icon.textContent = '📁';

    // Name
    const nameSpan = document.createElement('span');
    nameSpan.className = 'tree-dir-name';
    nameSpan.textContent = dir.name;

    header.appendChild(chevron);
    header.appendChild(icon);
    header.appendChild(nameSpan);

    // Children container
    const children = document.createElement('div');
    children.className = 'tree-children collapsed';

    for (const subdir of dir.dirs) {
        children.appendChild(buildDirNode(subdir, depth + 1));
    }
    for (const file of dir.files) {
        const fileNode = buildFileNode(file, depth + 1);
        children.appendChild(fileNode);
    }

    // Toggle
    header.addEventListener('click', () => {
        const isCollapsed = children.classList.contains('collapsed');
        children.classList.toggle('collapsed');
        chevron.classList.toggle('expanded', isCollapsed);
    });

    container.appendChild(header);
    container.appendChild(children);

    allDirElements.push({ el: container, header, children, chevron, name: dir.name });

    return container;
}

function buildFileNode(file, depth) {
    const el = document.createElement('div');
    el.className = 'tree-file';
    el.style.paddingLeft = (28 + (depth || 0) * 12) + 'px';
    el.dataset.path = file.path;
    el.dataset.kind = file.kind;
    el.dataset.size = file.size;
    el.tabIndex = 0;

    // Icon
    const iconWrap = document.createElement('span');
    iconWrap.className = 'tree-file-icon kind-' + file.kind;
    let svgStr;
    if (file.kind === '3mf') svgStr = cubeSVG();
    else if (file.kind === 'stl') svgStr = wireframeSVG();
    else svgStr = imageSVG();
    iconWrap.appendChild(createSVG(svgStr));

    // Name
    const nameSpan = document.createElement('span');
    nameSpan.className = 'tree-file-name';
    nameSpan.textContent = file.name;

    el.appendChild(iconWrap);
    el.appendChild(nameSpan);

    el.addEventListener('click', () => selectFile(el));

    allFileElements.push(el);
    return el;
}

// ─── File Selection ────────────────────────────────────────────────────────────

function selectFile(el) {
    if (selectedFileEl) {
        selectedFileEl.classList.remove('selected');
    }
    el.classList.add('selected');
    selectedFileEl = el;
    visibleFileIndex = allFileElements.indexOf(el);

    const path = el.dataset.path;
    const kind = el.dataset.kind;
    const size = parseInt(el.dataset.size, 10);

    currentFilePath = path;
    showingThumbnail = false;
    btnThumbnail.classList.remove('active');

    // Show preview pane
    previewEmpty.style.display = 'none';
    previewContent.style.display = 'flex';

    // Update info
    const filename = path.split('/').pop();
    infoFilename.textContent = filename;
    infoSize.textContent = formatSize(size);
    infoStats.textContent = '';

    // Show/hide thumbnail button
    btnThumbnail.style.display = kind === '3mf' ? 'inline-flex' : 'none';

    if (kind === '3mf' || kind === 'stl') {
        loadMesh(path);
    } else if (kind === 'image') {
        loadImage(path);
    }

    // Scroll file into view in tree
    el.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
}

// ─── Mesh Loading ──────────────────────────────────────────────────────────────

async function loadMesh(path) {
    // Dispose previous
    disposeMesh();

    // Show canvas, hide image/thumbnail
    previewCanvas.style.display = 'block';
    previewImage.style.display = 'none';
    previewThumbnail.style.display = 'none';

    // Show progress
    previewProgress.style.display = 'flex';
    progressBar.style.width = '0%';
    progressText.textContent = 'Loading…';

    try {
        const resp = await fetch('/api/mesh?path=' + encodeURIComponent(path));
        if (!resp.ok) {
            const err = await resp.json().catch(() => ({ error: 'Unknown error' }));
            throw new Error(err.error || 'Failed to load mesh');
        }

        const totalBytes = parseInt(resp.headers.get('content-length') || '0', 10);
        const reader = resp.body.getReader();
        let receivedBytes = 0;
        const chunks = [];

        while (true) {
            const { done, value } = await reader.read();
            if (done) break;
            chunks.push(value);
            receivedBytes += value.length;

            if (totalBytes > 0) {
                const pct = Math.min(100, (receivedBytes / totalBytes) * 100);
                progressBar.style.width = pct + '%';
                progressText.textContent = formatSize(receivedBytes) + ' / ' + formatSize(totalBytes);
            } else {
                progressText.textContent = formatSize(receivedBytes);
            }
        }

        // Check if user navigated away during load
        if (currentFilePath !== path) return;

        // Combine chunks
        const totalLength = chunks.reduce((a, c) => a + c.length, 0);
        const combined = new Uint8Array(totalLength);
        let offset = 0;
        for (const chunk of chunks) {
            combined.set(chunk, offset);
            offset += chunk.length;
        }

        // Parse wire format
        const view = new DataView(combined.buffer);
        const magic = view.getUint32(0, true);
        if (magic !== 0x4D455348) {
            throw new Error('Invalid mesh data (bad magic)');
        }

        const nVerts = view.getUint32(8, true);
        const nTris = view.getUint32(12, true);

        const positionsOffset = 16;
        const indicesOffset = positionsOffset + nVerts * 3 * 4;
        const colorsOffset = indicesOffset + nTris * 3 * 4;

        const positions = new Float32Array(combined.buffer, positionsOffset, nVerts * 3);
        const indices = new Uint32Array(combined.buffer, indicesOffset, nTris * 3);

        // Build geometry
        const geometry = new THREE.BufferGeometry();
        geometry.setAttribute('position', new THREE.BufferAttribute(positions, 3));
        geometry.setIndex(new THREE.BufferAttribute(indices, 1));
        geometry.computeVertexNormals();

        // Version 2+ wire format includes per-vertex linear-space colors.
        if (colorsOffset + nVerts * 3 * 4 <= combined.byteLength) {
            const colors = new Float32Array(combined.buffer, colorsOffset, nVerts * 3);
            geometry.setAttribute('color', new THREE.BufferAttribute(colors, 3));
        }

        const material = new THREE.MeshStandardMaterial({
            color: 0xffffff,
            vertexColors: true,
            roughness: 0.55,
            metalness: 0.05,
            flatShading: false,
        });

        const mesh = new THREE.Mesh(geometry, material);
        scene.add(mesh);
        currentMesh = mesh;

        // Frame camera to fit
        frameMesh(geometry);

        // Show stats
        infoStats.textContent = nVerts.toLocaleString() + ' vertices · ' + nTris.toLocaleString() + ' triangles';

        previewProgress.style.display = 'none';
        resizeRenderer();
    } catch (e) {
        previewProgress.style.display = 'none';
        showToast(e.message, true);
    }
}

function frameMesh(geometry) {
    geometry.computeBoundingBox();
    const box = geometry.boundingBox;
    const center = new THREE.Vector3();
    box.getCenter(center);
    const size = new THREE.Vector3();
    box.getSize(size);
    const maxDim = Math.max(size.x, size.y, size.z);
    const fov = camera.fov * (Math.PI / 180);
    let dist = maxDim / (2 * Math.tan(fov / 2));
    dist *= 1.5; // Padding

    controls.target.copy(center);
    camera.position.set(
        center.x + dist * 0.6,
        center.y - dist * 0.8,
        center.z + dist * 0.4
    );
    controls.update();
    camera.near = maxDim * 0.001;
    camera.far = maxDim * 100;
    camera.updateProjectionMatrix();
}

function disposeMesh() {
    if (currentMesh) {
        scene.remove(currentMesh);
        if (currentMesh.geometry) currentMesh.geometry.dispose();
        if (currentMesh.material) currentMesh.material.dispose();
        currentMesh = null;
    }
}

// ─── Image Loading ─────────────────────────────────────────────────────────────

function loadImage(path) {
    disposeMesh();
    previewCanvas.style.display = 'none';
    previewImage.style.display = 'block';
    previewThumbnail.style.display = 'none';
    previewProgress.style.display = 'none';
    previewImage.src = '/api/image?path=' + encodeURIComponent(path);
}

// ─── Thumbnail Toggle ──────────────────────────────────────────────────────────

function toggleThumbnail() {
    if (!currentFilePath) return;
    showingThumbnail = !showingThumbnail;
    btnThumbnail.classList.toggle('active', showingThumbnail);

    if (showingThumbnail) {
        previewCanvas.style.display = 'none';
        previewThumbnail.style.display = 'block';
        previewImage.style.display = 'none';
        previewThumbnail.src = '/api/thumbnail?path=' + encodeURIComponent(currentFilePath);
    } else {
        previewCanvas.style.display = 'block';
        previewThumbnail.style.display = 'none';
    }
}

// ─── Search ────────────────────────────────────────────────────────────────────

function filterTree(query) {
    const q = query.toLowerCase().trim();

    if (!q) {
        // Show everything, collapse all
        for (const f of allFileElements) {
            f.style.display = '';
        }
        for (const d of allDirElements) {
            d.el.style.display = '';
            d.children.classList.add('collapsed');
            d.chevron.classList.remove('expanded');
        }
        return;
    }

    // First pass: determine which files match
    const matchedFiles = new Set();
    for (const f of allFileElements) {
        const name = (f.dataset.path || '').toLowerCase();
        if (name.includes(q)) {
            matchedFiles.add(f);
            f.style.display = '';
        } else {
            f.style.display = 'none';
        }
    }

    // Second pass: show/expand dirs that contain matched files
    for (const d of allDirElements) {
        const hasMatch = hasMatchingDescendant(d.children, matchedFiles);
        if (hasMatch || d.name.toLowerCase().includes(q)) {
            d.el.style.display = '';
            d.children.classList.remove('collapsed');
            d.chevron.classList.add('expanded');
        } else {
            d.el.style.display = 'none';
        }
    }
}

function hasMatchingDescendant(container, matchedFiles) {
    const files = container.querySelectorAll('.tree-file');
    for (const f of files) {
        if (matchedFiles.has(f)) return true;
    }
    return false;
}

// ─── Keyboard Navigation ──────────────────────────────────────────────────────

function handleKeyNav(e) {
    const visibleFiles = allFileElements.filter(f => f.style.display !== 'none' && f.offsetParent !== null);
    if (visibleFiles.length === 0) return;

    if (e.key === 'ArrowDown') {
        e.preventDefault();
        visibleFileIndex = Math.min(visibleFileIndex + 1, visibleFiles.length - 1);
        if (visibleFileIndex < 0) visibleFileIndex = 0;
        selectFile(visibleFiles[visibleFileIndex]);
    } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        visibleFileIndex = Math.max(visibleFileIndex - 1, 0);
        selectFile(visibleFiles[visibleFileIndex]);
    } else if (e.key === 'Enter' && selectedFileEl) {
        e.preventDefault();
        selectFile(selectedFileEl);
    }
}

// ─── Splitter Drag ─────────────────────────────────────────────────────────────

function initSplitter() {
    let isDragging = false;
    let startX = 0;
    let startWidth = 0;

    splitter.addEventListener('mousedown', (e) => {
        isDragging = true;
        startX = e.clientX;
        startWidth = treePanel.offsetWidth;
        document.body.classList.add('dragging');
        splitter.classList.add('active');
        e.preventDefault();
    });

    document.addEventListener('mousemove', (e) => {
        if (!isDragging) return;
        const dx = e.clientX - startX;
        const newWidth = Math.max(200, Math.min(600, startWidth + dx));
        treePanel.style.width = newWidth + 'px';
        resizeRenderer();
    });

    document.addEventListener('mouseup', () => {
        if (!isDragging) return;
        isDragging = false;
        document.body.classList.remove('dragging');
        splitter.classList.remove('active');
    });
}

// ─── Toast ─────────────────────────────────────────────────────────────────────

function showToast(message, isError = false) {
    const toast = document.createElement('div');
    toast.className = 'toast' + (isError ? ' error' : '');
    toast.textContent = message;
    toastContainer.appendChild(toast);

    setTimeout(() => {
        toast.classList.add('fade-out');
        setTimeout(() => toast.remove(), 300);
    }, 4000);
}

// ─── Utilities ─────────────────────────────────────────────────────────────────

function formatSize(bytes) {
    if (bytes < 1024) return bytes + ' B';
    if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KB';
    if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MB';
    return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GB';
}

// ─── Event Bindings ────────────────────────────────────────────────────────────

btnDownload.addEventListener('click', () => {
    if (!currentFilePath) return;
    const a = document.createElement('a');
    a.href = '/api/download?path=' + encodeURIComponent(currentFilePath);
    a.download = '';
    a.click();
});

btnThumbnail.addEventListener('click', toggleThumbnail);

btnReset.addEventListener('click', () => {
    if (currentMesh && currentMesh.geometry) {
        frameMesh(currentMesh.geometry);
    }
});

searchInput.addEventListener('input', (e) => {
    filterTree(e.target.value);
});

document.addEventListener('keydown', (e) => {
    // Focus search on Ctrl+F / Cmd+F
    if ((e.ctrlKey || e.metaKey) && e.key === 'f') {
        e.preventDefault();
        searchInput.focus();
        searchInput.select();
        return;
    }

    // Keyboard nav in tree
    if (e.target === document.body || e.target.classList.contains('tree-file')) {
        handleKeyNav(e);
    }
});

// Resize handler
const resizeObserver = new ResizeObserver(() => {
    if (renderer) resizeRenderer();
});
resizeObserver.observe(previewViewport);

// ─── Init ──────────────────────────────────────────────────────────────────────

initThree();
initSplitter();
loadTree();
