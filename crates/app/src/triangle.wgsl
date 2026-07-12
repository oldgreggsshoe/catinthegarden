struct Time {
    seconds: f32,
}

@group(0) @binding(0)
var<uniform> time: Time;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) color: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let angle = time.seconds;
    let cosine = cos(angle);
    let sine = sin(angle);
    let rotated = vec2<f32>(
        input.position.x * cosine - input.position.y * sine,
        input.position.x * sine + input.position.y * cosine,
    );

    return VertexOutput(vec4<f32>(rotated, 0.0, 1.0), input.color);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);
}
