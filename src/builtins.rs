use std::sync::OnceLock;

mod inference;
pub use inference::return_type;

#[derive(Clone, Debug)]
pub struct Builtin {
    pub name: String,
    pub ty: String,
    pub kind: u32,
    pub signature: String,
    pub parameters: Vec<String>,
    pub description: String,
    pub stages: &'static str,
}

impl Builtin {
    pub fn available(&self, stage: Option<&str>) -> bool {
        stage.is_none_or(|s| {
            self.stages.is_empty() || self.stages.split_whitespace().any(|allowed| allowed == s)
        })
    }
}

pub fn all() -> &'static [Builtin] {
    static BUILTINS: OnceLock<Vec<Builtin>> = OnceLock::new();
    BUILTINS.get_or_init(|| {
        let mut items = Vec::new();
        for name in ["void", "bool", "int", "uint", "float", "double", "atomic_uint"] {
            add_type(&mut items, name.to_owned());
        }
        for prefix in ["", "b", "i", "u", "d"] {
            for n in 2..=4 { add_type(&mut items, format!("{prefix}vec{n}")); }
        }
        for prefix in ["", "d"] {
            for columns in 2..=4 {
                add_type(&mut items, format!("{prefix}mat{columns}"));
                for rows in 2..=4 { add_type(&mut items, format!("{prefix}mat{columns}x{rows}")); }
            }
        }
        for prefix in ["", "i", "u"] {
            for base in ["sampler", "image"] {
                for suffix in ["1D", "2D", "3D", "Cube", "2DRect", "1DArray", "2DArray", "CubeArray", "Buffer", "2DMS", "2DMSArray"] {
                    add_type(&mut items, format!("{prefix}{base}{suffix}"));
                }
            }
            for suffix in ["", "MS"] { add_type(&mut items, format!("{prefix}subpassInput{suffix}")); }
        }
        for suffix in ["1D", "2D", "Cube", "2DRect", "1DArray", "2DArray", "CubeArray"] {
            add_type(&mut items, format!("sampler{suffix}Shadow"));
        }
        for name in "const in out inout uniform buffer shared centroid sample patch flat smooth noperspective invariant precise highp mediump lowp coherent volatile restrict readonly writeonly precision layout struct if else for while do switch case default break continue return discard true false".split_whitespace() {
            items.push(Builtin { name: name.into(), ty: String::new(), kind: 14, signature: name.into(), parameters: vec![], description: "GLSL keyword or qualifier.".into(), stages: "" });
        }
        for (names, result, params, desc) in [
            ("radians degrees sin cos tan asin acos sinh cosh tanh asinh acosh atanh", "genType", "genType x", "Applies an angle or trigonometric operation component by component."),
            ("exp log exp2 log2 sqrt inversesqrt", "genType", "genType x", "Applies an exponential, logarithmic, or root operation component by component."),
            ("abs sign floor trunc round roundEven ceil fract", "genType", "genType x", "Applies a numeric operation component by component."),
            ("isnan isinf", "genBType", "genType x", "Tests each component for a special floating-point value."),
            ("floatBitsToInt", "genIType", "genType value", "Reinterprets floating-point bits as signed integer bits."),
            ("floatBitsToUint", "genUType", "genType value", "Reinterprets floating-point bits as unsigned integer bits."),
            ("intBitsToFloat", "genType", "genIType value", "Reinterprets signed integer bits as floating-point bits."),
            ("uintBitsToFloat", "genType", "genUType value", "Reinterprets unsigned integer bits as floating-point bits."),
            ("normalize", "genType", "genType x", "Returns a vector with unit length in the direction of x."),
            ("length", "float", "genType x", "Computes vector magnitude."),
            ("determinant", "float", "matN m", "Computes a square matrix's determinant."),
            ("inverse", "matN", "matN m", "Computes a square matrix's inverse."),
            ("transpose", "matRxC", "matCxR m", "Exchanges a matrix's rows and columns."),
            ("any all", "bool", "bvecN value", "Combines the components of a boolean vector."),
            ("not", "bvecN", "bvecN value", "Negates each boolean component."),
            ("bitCount findLSB findMSB", "genIType", "genIType value", "Queries an integer's bit representation."),
            ("bitfieldReverse", "genIType", "genIType value", "Reverses the bits in each component."),
            ("packUnorm2x16 packSnorm2x16 packHalf2x16", "uint", "vec2 value", "Packs two components into one unsigned integer."),
            ("packUnorm4x8 packSnorm4x8", "uint", "vec4 value", "Packs four components into one unsigned integer."),
            ("unpackUnorm2x16 unpackSnorm2x16 unpackHalf2x16", "vec2", "uint value", "Unpacks two components from an unsigned integer."),
            ("unpackUnorm4x8 unpackSnorm4x8", "vec4", "uint value", "Unpacks four components from an unsigned integer."),
            ("dFdx dFdy dFdxFine dFdyFine dFdxCoarse dFdyCoarse fwidth fwidthFine fwidthCoarse", "genType", "genType p", "Uses neighboring fragment invocations to estimate screen-space change."),
        ] {
            for name in names.split_whitespace() {
                add_function(&mut items, name, result, params, desc, if name.starts_with("dFd") || name.starts_with("fwidth") { "frag" } else { "" });
            }
        }
        for (name, result, params, desc) in [
            ("atan", "genType", "genType y, genType x", "Computes an angle; also accepts a single argument."),
            ("pow", "genType", "genType x, genType y", "Raises each component of x to a power."),
            ("mod", "genType", "genType x, genType y", "Computes the GLSL floating-point modulus."),
            ("modf", "genType", "genType x, out genType integral", "Separates a number's fractional and integral parts."),
            ("min", "genType", "genType x, genType y", "Selects the smaller value component by component."),
            ("max", "genType", "genType x, genType y", "Selects the larger value component by component."),
            ("clamp", "genType", "genType x, genType minVal, genType maxVal", "Constrains each value to an interval."),
            ("mix", "genType", "genType x, genType y, genType a", "Interpolates between x and y; boolean forms select components."),
            ("step", "genType", "genType edge, genType x", "Produces a step at the given edge."),
            ("smoothstep", "genType", "genType edge0, genType edge1, genType x", "Interpolates smoothly across an interval."),
            ("fma", "genType", "genType a, genType b, genType c", "Computes a multiply followed by an addition."),
            ("frexp", "genType", "genType x, out genIType exponent", "Splits a floating-point value into significand and exponent."),
            ("ldexp", "genType", "genType x, genIType exponent", "Scales a value by an integer power of two."),
            ("distance", "float", "genType p0, genType p1", "Computes the distance between two points."),
            ("dot", "float", "genType x, genType y", "Computes a vector dot product."),
            ("cross", "vec3", "vec3 x, vec3 y", "Computes a three-dimensional cross product."),
            ("faceforward", "genType", "genType N, genType I, genType Nref", "Orients a normal using a reference direction."),
            ("reflect", "genType", "genType I, genType N", "Computes a reflected direction."),
            ("refract", "genType", "genType I, genType N, float eta", "Computes a refracted direction."),
            ("matrixCompMult", "matN", "matN x, matN y", "Multiplies matrices component by component."),
            ("outerProduct", "matCxR", "vecR column, vecC row", "Builds a matrix from two vectors."),
            ("texture", "gvec4", "gsampler sampler, vecN P", "Samples a texture with implicit level of detail; some forms accept bias or comparison arguments."),
            ("textureLod", "gvec4", "gsampler sampler, vecN P, float lod", "Samples a texture at an explicit level of detail."),
            ("textureGrad", "gvec4", "gsampler sampler, vecN P, vecN dPdx, vecN dPdy", "Samples a texture using explicit coordinate derivatives."),
            ("textureProj", "gvec4", "gsampler sampler, vecN P", "Samples with projective texture coordinates."),
            ("textureProjOffset", "gvec4", "gsampler sampler, vecN P, ivecN offset", "Samples with projective coordinates and a texel offset."),
            ("textureProjLod", "gvec4", "gsampler sampler, vecN P, float lod", "Samples with projective coordinates at an explicit level of detail."),
            ("textureProjLodOffset", "gvec4", "gsampler sampler, vecN P, float lod, ivecN offset", "Samples with projective coordinates, explicit level of detail and a texel offset."),
            ("textureProjGrad", "gvec4", "gsampler sampler, vecN P, vecN dPdx, vecN dPdy", "Samples with projective coordinates and explicit derivatives."),
            ("textureProjGradOffset", "gvec4", "gsampler sampler, vecN P, vecN dPdx, vecN dPdy, ivecN offset", "Samples with projective coordinates, derivatives and a texel offset."),
            ("textureOffset", "gvec4", "gsampler sampler, vecN P, ivecN offset", "Samples with an integer texel offset."),
            ("textureLodOffset", "gvec4", "gsampler sampler, vecN P, float lod, ivecN offset", "Samples with explicit level of detail and a texel offset."),
            ("textureGradOffset", "gvec4", "gsampler sampler, vecN P, vecN dPdx, vecN dPdy, ivecN offset", "Samples with derivatives and a texel offset."),
            ("textureGather", "gvec4", "gsampler sampler, vecN P", "Collects a component from neighboring texels."),
            ("textureGatherOffset", "gvec4", "gsampler sampler, vecN P, ivec2 offset", "Gathers neighboring texels with an offset."),
            ("textureGatherOffsets", "gvec4", "gsampler sampler, vecN P, ivec2 offsets[4]", "Gathers texels at four offsets."),
            ("textureSize", "ivecN", "gsampler sampler, int lod", "Returns texture dimensions; forms without mipmaps omit lod."),
            ("textureQueryLod", "vec2", "gsampler sampler, vecN P", "Queries the level of detail used by texture sampling."),
            ("textureQueryLevels", "int", "gsampler sampler", "Returns the number of accessible mip levels."),
            ("textureSamples", "int", "gsampler2DMS sampler", "Returns the multisample texture's sample count."),
            ("texelFetch", "gvec4", "gsampler sampler, ivecN P, int lod", "Fetches a texel using integer coordinates."),
            ("texelFetchOffset", "gvec4", "gsampler sampler, ivecN P, int lod, ivecN offset", "Fetches a texel using integer coordinates and an offset."),
            ("imageSize", "ivecN", "gimage image", "Returns the image dimensions."),
            ("imageSamples", "int", "gimage2DMS image", "Returns the image's sample count."),
            ("imageLoad", "gvec4", "gimage image, ivecN P", "Reads an image texel."),
            ("imageStore", "void", "gimage image, ivecN P, gvec4 data", "Writes an image texel."),
            ("bitfieldExtract", "genIType", "genIType value, int offset, int bits", "Extracts a range of bits."),
            ("bitfieldInsert", "genIType", "genIType base, genIType insert, int offset, int bits", "Replaces a range of bits."),
            ("uaddCarry", "genUType", "genUType x, genUType y, out genUType carry", "Adds unsigned values and reports carry."),
            ("usubBorrow", "genUType", "genUType x, genUType y, out genUType borrow", "Subtracts unsigned values and reports borrow."),
            ("umulExtended", "void", "genUType x, genUType y, out genUType msb, out genUType lsb", "Computes both halves of an unsigned product."),
            ("imulExtended", "void", "genIType x, genIType y, out genIType msb, out genIType lsb", "Computes both halves of a signed product."),
            ("packDouble2x32", "double", "uvec2 value", "Combines two words into a double's bit representation."),
            ("unpackDouble2x32", "uvec2", "double value", "Splits a double's bit representation into two words."),
        ] { add_function(&mut items, name, result, params, desc, ""); }
        for name in ["lessThan", "lessThanEqual", "greaterThan", "greaterThanEqual", "equal", "notEqual"] {
            add_function(&mut items, name, "bvecN", "vecN x, vecN y", "Compares vector components.", "");
        }
        for name in ["atomicAdd", "atomicMin", "atomicMax", "atomicAnd", "atomicOr", "atomicXor", "atomicExchange"] {
            add_function(&mut items, name, "uint", "inout uint mem, uint data", "Atomically updates shared or buffer memory and returns its previous value. Signed forms are also available.", "");
        }
        add_function(&mut items, "atomicCompSwap", "uint", "inout uint mem, uint compare, uint data", "Atomically compares and conditionally replaces a memory value.", "");
        for name in ["imageAtomicAdd", "imageAtomicMin", "imageAtomicMax", "imageAtomicAnd", "imageAtomicOr", "imageAtomicXor", "imageAtomicExchange"] {
            add_function(&mut items, name, "uint", "uimage image, ivecN P, uint data", "Atomically updates an image texel and returns its previous value.", "");
        }
        add_function(&mut items, "imageAtomicCompSwap", "uint", "uimage image, ivecN P, uint compare, uint data", "Atomically compares and conditionally replaces an image texel.", "");
        for name in ["atomicCounter", "atomicCounterIncrement", "atomicCounterDecrement"] {
            add_function(&mut items, name, "uint", "atomic_uint counter", "Reads or updates an atomic counter.", "");
        }
        for name in ["barrier", "memoryBarrier", "memoryBarrierAtomicCounter", "memoryBarrierBuffer", "memoryBarrierShared", "memoryBarrierImage", "groupMemoryBarrier"] {
            add_function(&mut items, name, "void", "", "Synchronizes shader execution or memory visibility; permitted stages depend on the operation.", "");
        }
        for name in ["EmitVertex", "EndPrimitive"] { add_function(&mut items, name, "void", "", "Controls geometry shader output.", "geom"); }
        for name in ["EmitStreamVertex", "EndStreamPrimitive"] { add_function(&mut items, name, "void", "int stream", "Controls geometry output on a stream.", "geom"); }
        add_function(&mut items, "subpassLoad", "gvec4", "gsubpassInput input", "Reads an input attachment in a Vulkan fragment shader.", "frag");
        for (name, ty, stages) in [
            ("gl_Position", "vec4", "vert tesc tese geom mesh"), ("gl_PointSize", "float", "vert tesc tese geom mesh"),
            ("gl_ClipDistance", "float[]", "vert tesc tese geom frag"), ("gl_CullDistance", "float[]", "vert tesc tese geom frag"),
            ("gl_VertexID", "int", "vert"), ("gl_InstanceID", "int", "vert"), ("gl_VertexIndex", "int", "vert"), ("gl_InstanceIndex", "int", "vert"),
            ("gl_DrawID", "int", "vert"), ("gl_BaseVertex", "int", "vert"), ("gl_BaseInstance", "int", "vert"),
            ("gl_FragCoord", "vec4", "frag"), ("gl_FrontFacing", "bool", "frag"), ("gl_PointCoord", "vec2", "frag"),
            ("gl_FragDepth", "float", "frag"), ("gl_SampleID", "int", "frag"), ("gl_SamplePosition", "vec2", "frag"),
            ("gl_SampleMaskIn", "int[]", "frag"), ("gl_SampleMask", "int[]", "frag"), ("gl_HelperInvocation", "bool", "frag"),
            ("gl_NumWorkGroups", "uvec3", "comp task mesh"), ("gl_WorkGroupSize", "uvec3", "comp task mesh"),
            ("gl_WorkGroupID", "uvec3", "comp task mesh"), ("gl_LocalInvocationID", "uvec3", "comp task mesh"),
            ("gl_GlobalInvocationID", "uvec3", "comp task mesh"), ("gl_LocalInvocationIndex", "uint", "comp task mesh"),
            ("gl_InvocationID", "int", "tesc geom"), ("gl_PrimitiveID", "int", "tesc tese geom frag"),
            ("gl_PrimitiveIDIn", "int", "geom"), ("gl_Layer", "int", "geom frag"), ("gl_ViewportIndex", "int", "geom frag"),
            ("gl_TessCoord", "vec3", "tese"), ("gl_PatchVerticesIn", "int", "tesc tese"),
            ("gl_TessLevelOuter", "float[4]", "tesc tese"), ("gl_TessLevelInner", "float[2]", "tesc tese"),
        ] {
            items.push(Builtin { name: name.into(), ty: ty.into(), kind: 6, signature: format!("{ty} {name}"), parameters: Vec::new(), description: "GLSL built-in shader variable. Availability depends on the shader stage, version, and target API.".into(), stages });
        }
        items
    })
}

fn add_type(items: &mut Vec<Builtin>, name: String) {
    items.push(Builtin {
        ty: name.clone(),
        signature: name.clone(),
        name,
        kind: 7,
        parameters: vec![],
        description: "GLSL built-in type. Numeric, vector and matrix types also have constructors."
            .into(),
        stages: "",
    });
}

fn add_function(
    items: &mut Vec<Builtin>,
    name: &str,
    ty: &str,
    params: &str,
    description: &str,
    stages: &'static str,
) {
    items.push(Builtin { name: name.into(), ty: ty.into(), kind: 3, signature: format!("{ty} {name}({params})"), parameters: if params.is_empty() { vec![] } else { params.split(", ").map(str::to_owned).collect() }, description: format!("{description}\n\nGeneric type names summarize overloads. Availability depends on GLSL version, extensions and target API."), stages });
}
