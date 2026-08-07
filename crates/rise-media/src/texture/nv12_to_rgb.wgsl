// YUV -> RGB on the GPU.
//
// The CPU never touches pixel data. A hardware decoder returns biplanar YUV;
// converting it to RGBA on the CPU before upload is the per-frame memcpy this
// whole module exists to avoid.
//
// Three sampling paths because three plane layouts reach us: NV12 and P010 are
// biplanar (luma, interleaved chroma), I420 is triplanar. The matrix differs by
// colour space, not by plane layout, so it is a uniform rather than a branch.

struct ConversionParams {
    // Row 0..2 of the YUV -> RGB matrix. Row 3 is implicit.
    matrix: mat3x3<f32>,
    // Subtracted from (Y, U, V) before the matrix. Limited range video is
    // (16/255, 128/255, 128/255); full range is (0, 0.5, 0.5). Getting this
    // wrong is the classic washed-out-blacks bug, not a crash, so it is worth
    // being explicit about.
    offset: vec3<f32>,
    // 0 = NV12/P010 biplanar, 1 = I420 triplanar.
    plane_layout: u32,
    _padding: u32,
}

@group(0) @binding(0) var<uniform> params: ConversionParams;

// Biplanar: nv12 and p010 both bind here. p010's 10 bits arrive in the high
// bits of a 16-bit sample, which the R16Unorm/Rg16Unorm view already
// normalises, so no shift is needed in the shader.
@group(0) @binding(1) var luma_plane: texture_2d<f32>;
@group(0) @binding(2) var chroma_plane: texture_2d<f32>;

// Triplanar: i420's third plane. Bound to a 1x1 dummy on the biplanar path so
// the layout stays identical and one pipeline serves both.
@group(0) @binding(3) var chroma_v_plane: texture_2d<f32>;

@group(0) @binding(4) var frame_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// A full-screen triangle, not a quad: no vertex buffer, and no seam along the
// diagonal where two triangles would meet.
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    var out: VertexOutput;

    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    out.uv = uv;
    out.position = vec4<f32>(uv * vec2<f32>(2.0, -2.0) + vec2<f32>(-1.0, 1.0), 0.0, 1.0);

    return out;
}

const PLANE_LAYOUT_BIPLANAR: u32 = 0u;
const PLANE_LAYOUT_TRIPLANAR: u32 = 1u;

fn sample_chroma(uv: vec2<f32>) -> vec2<f32> {
    if params.plane_layout == PLANE_LAYOUT_TRIPLANAR {
        // i420: U and V are separate single-channel planes.
        let u = textureSample(chroma_plane, frame_sampler, uv).r;
        let v = textureSample(chroma_v_plane, frame_sampler, uv).r;
        return vec2<f32>(u, v);
    }

    // nv12 and p010: U and V interleaved in one two-channel plane.
    return textureSample(chroma_plane, frame_sampler, uv).rg;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let y = textureSample(luma_plane, frame_sampler, in.uv).r;
    let uv = sample_chroma(in.uv);

    let yuv = vec3<f32>(y, uv.x, uv.y) - params.offset;
    let rgb = params.matrix * yuv;

    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
