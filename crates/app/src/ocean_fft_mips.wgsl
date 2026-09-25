// 2x2 box downsample of the FFT field, one mip level per dispatch.
@group(0) @binding(0) var source: texture_2d_array<f32>;
@group(0) @binding(1) var dest: texture_storage_2d_array<rgba16float, write>;

@compute @workgroup_size(8, 8, 1)
fn downsample(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(dest);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let base = vec2<i32>(id.xy) * 2;
    let layer = i32(id.z);
    let sum = textureLoad(source, base, layer, 0)
        + textureLoad(source, base + vec2<i32>(1, 0), layer, 0)
        + textureLoad(source, base + vec2<i32>(0, 1), layer, 0)
        + textureLoad(source, base + vec2<i32>(1, 1), layer, 0);
    textureStore(dest, vec2<i32>(id.xy), layer, sum * 0.25);
}
