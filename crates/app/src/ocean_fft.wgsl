// GPU FFT ocean wave field (Tessendorf). Four cascades (three wind, one swell) of 256x256, each with
// two packed complex FFTs carrying h, Dx, Dz (P0 = h + i Dx, P1 = Dz).
const N: u32 = 256u;
const HALF_N: f32 = 128.0;
const TAU: f32 = 6.283185307179586;
const GRAVITY: f32 = 9.81;

struct Params {
    time: f32,
    pad0: f32,
    pad1: f32,
    pad2: f32,
    tile: array<vec4<f32>, 4>,
}

@group(0) @binding(0) var<uniform> params: Params;
// (h0(k).re, h0(k).im, h0(-k).re, h0(-k).im), 3 cascades.
@group(0) @binding(1) var<storage, read> h0: array<vec4<f32>>;
// 6 arrays (cascade * 2 + pack) of N*N complex values.
@group(0) @binding(2) var<storage, read_write> spec: array<vec2<f32>>;
// One layer per cascade: (h, Dx, Dz, 0).
@group(0) @binding(3) var field: texture_storage_2d_array<rgba16float, write>;

fn cmul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

fn times_i(v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(-v.y, v.x);
}

@compute @workgroup_size(8, 8, 1)
fn evolve(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = id.z;
    let cell = id.y * N + id.x;
    let src = h0[c * N * N + cell];
    let length = params.tile[c].x;
    let kx = TAU * (f32(id.x) - HALF_N) / length;
    let kz = TAU * (f32(id.y) - HALF_N) / length;
    let kl = max(sqrt(kx * kx + kz * kz), 1e-6);
    let phase = sqrt(GRAVITY * kl) * params.time;
    let e = vec2<f32>(cos(phase), sin(phase));
    let h = cmul(src.xy, e) + cmul(vec2<f32>(src.z, -src.w), vec2<f32>(e.x, -e.y));
    let sx = kx / kl;
    let sz = kz / kl;
    // D = -i (k/|k|) h. Slopes and the fold Jacobian are finite differences
    // of these textures at shading time.
    let dx = vec2<f32>(sx * h.y, -sx * h.x);
    let dz = vec2<f32>(sz * h.y, -sz * h.x);
    let base = c * 2u * N * N + cell;
    spec[base] = h + times_i(dx);
    spec[base + N * N] = dz;
}

var<workgroup> line: array<vec2<f32>, 256>;

fn run_fft(b: u32) {
    for (var s = 0u; s < 8u; s = s + 1u) {
        workgroupBarrier();
        let half = 1u << s;
        let j = b & (half - 1u);
        let i0 = ((b >> s) << (s + 1u)) + j;
        let i1 = i0 + half;
        let angle = TAU * f32(j) / f32(half * 2u);
        let w = vec2<f32>(cos(angle), sin(angle));
        let t = cmul(w, line[i1]);
        let u = line[i0];
        line[i0] = u + t;
        line[i1] = u - t;
    }
    workgroupBarrier();
}

@compute @workgroup_size(128, 1, 1)
fn fft_rows(@builtin(local_invocation_index) l: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let base = wg.y * N * N + wg.x * N;
    line[reverseBits(l) >> 24u] = spec[base + l];
    line[reverseBits(l + 128u) >> 24u] = spec[base + l + 128u];
    run_fft(l);
    spec[base + l] = line[l];
    spec[base + l + 128u] = line[l + 128u];
}

@compute @workgroup_size(128, 1, 1)
fn fft_cols(@builtin(local_invocation_index) l: u32, @builtin(workgroup_id) wg: vec3<u32>) {
    let base = wg.y * N * N + wg.x;
    line[reverseBits(l) >> 24u] = spec[base + l * N];
    line[reverseBits(l + 128u) >> 24u] = spec[base + (l + 128u) * N];
    run_fft(l);
    spec[base + l * N] = line[l];
    spec[base + (l + 128u) * N] = line[l + 128u];
}

@compute @workgroup_size(8, 8, 1)
fn assemble(@builtin(global_invocation_id) id: vec3<u32>) {
    let c = id.z;
    let cell = id.y * N + id.x;
    // The spectrum is stored centred, so the spatial result alternates sign.
    var sign = 1.0;
    if (((id.x + id.y) & 1u) == 1u) {
        sign = -1.0;
    }
    let base = c * 2u * N * N + cell;
    let p0 = spec[base];
    let p1 = spec[base + N * N];
    textureStore(field, vec2<i32>(i32(id.x), i32(id.y)), i32(c), sign * vec4<f32>(p0.x, p0.y, p1.x, 0.0));
}
