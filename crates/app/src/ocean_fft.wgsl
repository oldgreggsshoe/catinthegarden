// Tessendorf FFT ocean: spectrum -> row IFFT -> column IFFT -> resolve -> mips.
// FFT_N, FFT_LOG2_N and FFT_CASCADE_SIZES are prepended by ocean_fft.rs.
// Every pass works on all cascades at once through the array layer.
//
// Eight real fields travel as four complex pairs. Each field is real in space,
// so its spectrum is Hermitian and F1 + i F2 inverse-transforms to f1 + i f2:
//   a.xy = Dx + i h       a.zw = Dz + i dDx/dz
//   b.xy = dh/dx + i dh/dz  b.zw = dDx/dx + i dDz/dz

struct FftParams {
    time: f32,
    choppiness: f32,
    padding: vec2<f32>,
}

@group(0) @binding(0) var<uniform> fft_params: FftParams;
@group(0) @binding(1) var h0_texture: texture_2d_array<f32>;
@group(0) @binding(2) var omega_texture: texture_2d_array<f32>;
@group(0) @binding(3) var spectrum_a: texture_storage_2d_array<rgba32float, write>;
@group(0) @binding(4) var spectrum_b: texture_storage_2d_array<rgba32float, write>;

@group(0) @binding(5) var butterfly_source_a: texture_2d_array<f32>;
@group(0) @binding(6) var butterfly_source_b: texture_2d_array<f32>;
@group(0) @binding(7) var butterfly_target_a: texture_storage_2d_array<rgba32float, write>;
@group(0) @binding(8) var butterfly_target_b: texture_storage_2d_array<rgba32float, write>;

@group(0) @binding(9) var resolve_source_a: texture_2d_array<f32>;
@group(0) @binding(10) var resolve_source_b: texture_2d_array<f32>;
@group(0) @binding(11) var displacement_out: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(12) var derivatives_out: texture_storage_2d_array<rgba16float, write>;

@group(0) @binding(13) var mip_source_displacement: texture_2d_array<f32>;
@group(0) @binding(14) var mip_source_derivatives: texture_2d_array<f32>;
@group(0) @binding(15) var mip_target_displacement: texture_storage_2d_array<rgba16float, write>;
@group(0) @binding(16) var mip_target_derivatives: texture_storage_2d_array<rgba16float, write>;

fn complex_mul(a: vec2<f32>, b: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(a.x * b.x - a.y * b.y, a.x * b.y + a.y * b.x);
}

// i * z
fn times_i(z: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(-z.y, z.x);
}

@compute @workgroup_size(8, 8, 1)
fn cs_spectrum(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= FFT_N || id.y >= FFT_N {
        return;
    }
    let layer = i32(id.z);
    let texel = vec2<i32>(id.xy);
    let h0 = textureLoad(h0_texture, texel, layer, 0);
    let omega = textureLoad(omega_texture, texel, layer, 0).r;
    let size = FFT_CASCADE_SIZES[id.z];
    let half = i32(FFT_N / 2u);
    let k = vec2<f32>(f32(texel.x - half), f32(texel.y - half)) * (6.28318530718 / size);
    // `time` is already reduced modulo the repeat period on the CPU.
    let phase = omega * fft_params.time;
    let rotation = vec2<f32>(cos(phase), sin(phase));
    // h(k, t) = h0(k) e^{-i w t} + conj(h0(-k)) e^{i w t}
    let h = complex_mul(h0.xy, vec2<f32>(rotation.x, -rotation.y))
        + complex_mul(h0.zw, rotation);
    let wave_number = length(k);
    var direction = vec2<f32>(0.0);
    var inverse_k = 0.0;
    if wave_number > 1.0e-6 {
        direction = k / wave_number;
        inverse_k = 1.0 / wave_number;
    }
    let lambda = fft_params.choppiness;
    // D = +i k/|k| h lambda: water moves toward a rising crest, as in a
    // Gerstner wave (h = A cos kx gives D = -A sin kx), so crests sharpen and
    // troughs flatten. Tessendorf's paper writes x + lambda D with the other
    // sign; taken literally it rounds the crests and points the troughs,
    // which read as boiling water. Derivatives pick up another factor i k.
    let i_h = times_i(h);
    let dx = i_h * (direction.x * lambda);
    let dz = i_h * (direction.y * lambda);
    let dxx = -h * (k.x * k.x * inverse_k * lambda);
    let dzz = -h * (k.y * k.y * inverse_k * lambda);
    let dxz = -h * (k.x * k.y * inverse_k * lambda);
    let hx = times_i(h) * k.x;
    let hz = times_i(h) * k.y;
    let a = vec4<f32>(dx + times_i(h), dz + times_i(dxz));
    let b = vec4<f32>(hx + times_i(hz), dxx + times_i(dzz));
    textureStore(spectrum_a, texel, layer, a);
    textureStore(spectrum_b, texel, layer, b);
}

var<workgroup> row_a: array<vec4<f32>, FFT_N>;
var<workgroup> row_b: array<vec4<f32>, FFT_N>;

fn bit_reverse(index: u32) -> u32 {
    return reverseBits(index) >> (32u - FFT_LOG2_N);
}

fn butterfly_pair(value: vec4<f32>, twiddle: vec2<f32>) -> vec4<f32> {
    return vec4<f32>(complex_mul(value.xy, twiddle), complex_mul(value.zw, twiddle));
}

// In-place radix-2 inverse transform of row_a/row_b, one butterfly per thread
// per stage. The caller has loaded both rows in bit-reversed order.
fn inverse_fft_shared(thread: u32) {
    for (var stage = 0u; stage < FFT_LOG2_N; stage = stage + 1u) {
        let half = 1u << stage;
        let group = thread / half;
        let position = thread % half;
        let first = group * half * 2u + position;
        let second = first + half;
        let angle = 3.14159265359 * f32(position) / f32(half);
        let twiddle = vec2<f32>(cos(angle), sin(angle));
        let a0 = row_a[first];
        let a1 = butterfly_pair(row_a[second], twiddle);
        let b0 = row_b[first];
        let b1 = butterfly_pair(row_b[second], twiddle);
        row_a[first] = a0 + a1;
        row_a[second] = a0 - a1;
        row_b[first] = b0 + b1;
        row_b[second] = b0 - b1;
        workgroupBarrier();
    }
}

@compute @workgroup_size(128, 1, 1)
fn cs_fft_rows(
    @builtin(local_invocation_id) local: vec3<u32>,
    @builtin(workgroup_id) group: vec3<u32>,
) {
    let row = i32(group.x);
    let layer = i32(group.z);
    for (var element = 0u; element < 2u; element = element + 1u) {
        let index = local.x + element * 128u;
        let target_index = bit_reverse(index);
        row_a[target_index] = textureLoad(butterfly_source_a, vec2<i32>(i32(index), row), layer, 0);
        row_b[target_index] = textureLoad(butterfly_source_b, vec2<i32>(i32(index), row), layer, 0);
    }
    workgroupBarrier();
    inverse_fft_shared(local.x);
    for (var element = 0u; element < 2u; element = element + 1u) {
        let index = local.x + element * 128u;
        textureStore(butterfly_target_a, vec2<i32>(i32(index), row), layer, row_a[index]);
        textureStore(butterfly_target_b, vec2<i32>(i32(index), row), layer, row_b[index]);
    }
}

@compute @workgroup_size(128, 1, 1)
fn cs_fft_columns(
    @builtin(local_invocation_id) local: vec3<u32>,
    @builtin(workgroup_id) group: vec3<u32>,
) {
    let column = i32(group.x);
    let layer = i32(group.z);
    for (var element = 0u; element < 2u; element = element + 1u) {
        let index = local.x + element * 128u;
        let target_index = bit_reverse(index);
        row_a[target_index] = textureLoad(butterfly_source_a, vec2<i32>(column, i32(index)), layer, 0);
        row_b[target_index] = textureLoad(butterfly_source_b, vec2<i32>(column, i32(index)), layer, 0);
    }
    workgroupBarrier();
    inverse_fft_shared(local.x);
    for (var element = 0u; element < 2u; element = element + 1u) {
        let index = local.x + element * 128u;
        textureStore(butterfly_target_a, vec2<i32>(column, i32(index)), layer, row_a[index]);
        textureStore(butterfly_target_b, vec2<i32>(column, i32(index)), layer, row_b[index]);
    }
}

@compute @workgroup_size(8, 8, 1)
fn cs_resolve(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= FFT_N || id.y >= FFT_N {
        return;
    }
    let texel = vec2<i32>(id.xy);
    let layer = i32(id.z);
    // Storing k = n - N/2 at index n multiplies the transform by (-1)^(x+y).
    let sign = select(-1.0, 1.0, ((id.x + id.y) & 1u) == 0u);
    let a = textureLoad(resolve_source_a, texel, layer, 0) * sign;
    let b = textureLoad(resolve_source_b, texel, layer, 0) * sign;
    // displacement: (Dx, h, Dz, dDx/dz); derivatives: (dh/dx, dh/dz, dDx/dx, dDz/dz)
    textureStore(displacement_out, texel, layer, vec4<f32>(a.x, a.y, a.z, a.w));
    textureStore(derivatives_out, texel, layer, b);
}

@compute @workgroup_size(8, 8, 1)
fn cs_mip(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(mip_target_displacement);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let layer = i32(id.z);
    let base = vec2<i32>(id.xy) * 2;
    var displacement = vec4<f32>(0.0);
    var derivatives = vec4<f32>(0.0);
    for (var y = 0; y < 2; y = y + 1) {
        for (var x = 0; x < 2; x = x + 1) {
            displacement += textureLoad(mip_source_displacement, base + vec2<i32>(x, y), layer, 0);
            derivatives += textureLoad(mip_source_derivatives, base + vec2<i32>(x, y), layer, 0);
        }
    }
    textureStore(mip_target_displacement, vec2<i32>(id.xy), layer, displacement * 0.25);
    textureStore(mip_target_derivatives, vec2<i32>(id.xy), layer, derivatives * 0.25);
}
