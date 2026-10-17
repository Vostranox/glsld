pub fn return_type(name: &str, arguments: &[Option<String>]) -> Option<String> {
    let argument = |at: usize| arguments.get(at)?.as_deref();
    let numeric = |at, allowed: &[&str]| {
        let ty = argument(at)?;
        let (scalar, _) = numeric_shape(ty)?;
        allowed.contains(&scalar).then(|| ty.to_owned())
    };
    let float = &["float", "double"][..];
    let integer = &["int", "uint"][..];
    let number = &["float", "double", "int", "uint"][..];
    match (name, arguments.len()) {
        (
            "radians" | "degrees" | "sin" | "cos" | "tan" | "asin" | "acos" | "sinh" | "cosh"
            | "tanh" | "asinh" | "acosh" | "atanh" | "exp" | "log" | "exp2" | "log2" | "dFdx"
            | "dFdy" | "dFdxFine" | "dFdyFine" | "dFdxCoarse" | "dFdyCoarse" | "fwidth"
            | "fwidthFine" | "fwidthCoarse",
            1,
        )
        | ("atan", 1) => numeric(0, &["float"]),
        ("atan" | "pow", 2) => {
            compatible_arguments(arguments, 0, &[1], false)?;
            numeric(0, &["float"])
        }
        (
            "sqrt" | "inversesqrt" | "floor" | "trunc" | "round" | "roundEven" | "ceil" | "fract"
            | "normalize",
            1,
        ) => numeric(0, float),
        ("mod", 2) => {
            compatible_arguments(arguments, 0, &[1], true)?;
            numeric(0, float)
        }
        ("modf" | "reflect", 2) => {
            compatible_arguments(arguments, 0, &[1], false)?;
            numeric(0, float)
        }
        ("frexp" | "ldexp", 2) => {
            let (_, size) = numeric_shape(argument(0)?)?;
            (argument(1)? == with_scalar("int", size)).then_some(())?;
            numeric(0, float)
        }
        ("fma" | "faceforward", 3) => {
            compatible_arguments(arguments, 0, &[1, 2], false)?;
            numeric(0, float)
        }
        ("refract", 3) => {
            compatible_arguments(arguments, 0, &[1], false)?;
            compatible_arguments(arguments, 0, &[2], true)?;
            (numeric_shape(argument(2)?)?.1 == 1).then_some(())?;
            numeric(0, float)
        }
        ("abs" | "sign", 1) => numeric(0, &["float", "double", "int"]),
        ("min" | "max", 2) => {
            compatible_arguments(arguments, 0, &[1], true)?;
            numeric(0, number)
        }
        ("clamp", 3) => {
            compatible_arguments(arguments, 0, &[1, 2], true)?;
            (numeric_shape(argument(1)?)?.1 == numeric_shape(argument(2)?)?.1).then_some(())?;
            numeric(0, number)
        }
        ("mix", 3) => {
            compatible_arguments(arguments, 0, &[1], false)?;
            let ty = argument(0)?;
            let (scalar, size) = numeric_shape(ty)?;
            let (selector, selector_size) = numeric_shape(argument(2)?)?;
            if selector == "bool" {
                (selector_size == size).then(|| ty.into())
            } else if float.contains(&scalar) {
                compatible_arguments(arguments, 0, &[2], true)?;
                numeric(0, float)
            } else {
                None
            }
        }
        ("step", 2) => {
            compatible_arguments(arguments, 1, &[0], true)?;
            numeric(1, float)
        }
        ("smoothstep", 3) => {
            compatible_arguments(arguments, 2, &[0, 1], true)?;
            (numeric_shape(argument(0)?)?.1 == numeric_shape(argument(1)?)?.1).then_some(())?;
            numeric(2, float)
        }
        ("isnan" | "isinf", 1) => {
            numeric(0, float)?;
            Some(with_scalar("bool", numeric_shape(argument(0)?)?.1))
        }
        ("floatBitsToInt" | "floatBitsToUint", 1) => {
            numeric(0, &["float"])?;
            let scalar = if name == "floatBitsToInt" {
                "int"
            } else {
                "uint"
            };
            Some(with_scalar(scalar, numeric_shape(argument(0)?)?.1))
        }
        ("intBitsToFloat" | "uintBitsToFloat", 1) => {
            let scalar = if name == "intBitsToFloat" {
                "int"
            } else {
                "uint"
            };
            numeric(0, &[scalar])?;
            Some(with_scalar("float", numeric_shape(argument(0)?)?.1))
        }
        ("length", 1) => {
            numeric(0, float)?;
            Some(numeric_shape(argument(0)?)?.0.into())
        }
        ("distance" | "dot", 2) => {
            compatible_arguments(arguments, 0, &[1], false)?;
            numeric(0, float)?;
            Some(numeric_shape(argument(0)?)?.0.into())
        }
        ("cross", 2) => {
            compatible_arguments(arguments, 0, &[1], false)?;
            let ty = argument(0)?;
            matches!(ty, "vec3" | "dvec3").then(|| ty.into())
        }
        ("lessThan" | "lessThanEqual" | "greaterThan" | "greaterThanEqual", 2)
        | ("equal" | "notEqual", 2) => {
            compatible_arguments(arguments, 0, &[1], false)?;
            let (scalar, size) = numeric_shape(argument(0)?)?;
            (size > 1 && (scalar != "bool" || matches!(name, "equal" | "notEqual")))
                .then(|| with_scalar("bool", size))
        }
        ("any" | "all", 1) => {
            let (scalar, size) = numeric_shape(argument(0)?)?;
            (scalar == "bool" && size > 1).then(|| "bool".into())
        }
        ("not", 1) => {
            let ty = numeric(0, &["bool"])?;
            (numeric_shape(&ty)?.1 > 1).then_some(ty)
        }
        ("bitCount" | "findLSB" | "findMSB", 1) => {
            numeric(0, integer)?;
            Some(with_scalar("int", numeric_shape(argument(0)?)?.1))
        }
        ("bitfieldReverse", 1) | ("bitfieldExtract", 3) | ("bitfieldInsert", 4) => {
            numeric(0, integer)
        }
        ("uaddCarry" | "usubBorrow", 3) => numeric(0, &["uint"]),
        ("umulExtended" | "imulExtended", 4) => Some("void".into()),
        ("determinant" | "inverse" | "transpose", 1) | ("matrixCompMult", 2) => {
            let (scalar, columns, rows) = matrix_shape(argument(0)?)?;
            match name {
                "determinant" if columns == rows => Some(scalar.into()),
                "inverse" if columns == rows => Some(matrix_type(scalar, columns, rows)),
                "transpose" => Some(matrix_type(scalar, rows, columns)),
                "matrixCompMult" => {
                    let other = matrix_shape(argument(1)?)?;
                    (other == (scalar, columns, rows)).then(|| matrix_type(scalar, columns, rows))
                }
                _ => None,
            }
        }
        ("outerProduct", 2) => {
            let (scalar, rows) = numeric_shape(argument(0)?)?;
            let (other_scalar, columns) = numeric_shape(argument(1)?)?;
            (float.contains(&scalar) && scalar == other_scalar && rows > 1 && columns > 1)
                .then(|| matrix_type(scalar, columns, rows))
        }
        ("packUnorm2x16" | "packSnorm2x16" | "packHalf2x16", 1) => {
            fixed(argument(0), "vec2", "uint")
        }
        ("packUnorm4x8" | "packSnorm4x8", 1) => fixed(argument(0), "vec4", "uint"),
        ("unpackUnorm2x16" | "unpackSnorm2x16" | "unpackHalf2x16", 1) => {
            fixed(argument(0), "uint", "vec2")
        }
        ("unpackUnorm4x8" | "unpackSnorm4x8", 1) => fixed(argument(0), "uint", "vec4"),
        ("packDouble2x32", 1) => fixed(argument(0), "uvec2", "double"),
        ("unpackDouble2x32", 1) => fixed(argument(0), "double", "uvec2"),
        (
            "atomicAdd" | "atomicMin" | "atomicMax" | "atomicAnd" | "atomicOr" | "atomicXor"
            | "atomicExchange",
            2,
        )
        | ("atomicCompSwap", 3) => {
            let ty = argument(0)?;
            integer.contains(&ty).then(|| ty.into())
        }
        ("atomicCounter" | "atomicCounterIncrement" | "atomicCounterDecrement", 1) => {
            fixed(argument(0), "atomic_uint", "uint")
        }
        (
            "barrier"
            | "memoryBarrier"
            | "memoryBarrierAtomicCounter"
            | "memoryBarrierBuffer"
            | "memoryBarrierShared"
            | "memoryBarrierImage"
            | "groupMemoryBarrier"
            | "EmitVertex"
            | "EndPrimitive",
            0,
        )
        | ("EmitStreamVertex" | "EndStreamPrimitive", 1) => Some("void".into()),
        _ => resource_return_type(name, argument(0)?, arguments.len()),
    }
}

fn fixed(argument: Option<&str>, expected: &str, result: &str) -> Option<String> {
    (argument? == expected).then(|| result.into())
}

// Keep unknown operands and result-changing promotions unresolved.
fn compatible_arguments(
    arguments: &[Option<String>],
    result: usize,
    others: &[usize],
    allow_scalar: bool,
) -> Option<()> {
    let (scalar, size) = numeric_shape(arguments.get(result)?.as_deref()?)?;
    for &index in others {
        let (other, other_size) = numeric_shape(arguments.get(index)?.as_deref()?)?;
        if other_size != size && !(allow_scalar && other_size == 1) {
            return None;
        }
        if other != scalar
            && !matches!(
                (other, scalar),
                ("int", "uint" | "float" | "double")
                    | ("uint", "float" | "double")
                    | ("float", "double")
            )
        {
            return None;
        }
    }
    Some(())
}

fn numeric_shape(ty: &str) -> Option<(&'static str, usize)> {
    for (scalar, prefix) in [
        ("float", "vec"),
        ("double", "dvec"),
        ("int", "ivec"),
        ("uint", "uvec"),
        ("bool", "bvec"),
    ] {
        if ty == scalar {
            return Some((scalar, 1));
        }
        if let Some(size) = ty.strip_prefix(prefix).and_then(dimension) {
            return Some((scalar, size));
        }
    }
    None
}

fn dimension(value: &str) -> Option<usize> {
    match value {
        "2" => Some(2),
        "3" => Some(3),
        "4" => Some(4),
        _ => None,
    }
}

fn with_scalar(scalar: &str, size: usize) -> String {
    if size == 1 {
        return scalar.into();
    }
    let prefix = match scalar {
        "double" => "d",
        "int" => "i",
        "uint" => "u",
        "bool" => "b",
        _ => "",
    };
    format!("{prefix}vec{size}")
}

fn matrix_shape(ty: &str) -> Option<(&'static str, usize, usize)> {
    let (scalar, shape) = if let Some(shape) = ty.strip_prefix("dmat") {
        ("double", shape)
    } else {
        ("float", ty.strip_prefix("mat")?)
    };
    let (columns, rows) = shape.split_once('x').unwrap_or((shape, shape));
    Some((scalar, dimension(columns)?, dimension(rows)?))
}

fn matrix_type(scalar: &str, columns: usize, rows: usize) -> String {
    let prefix = if scalar == "double" { "d" } else { "" };
    if columns == rows {
        format!("{prefix}mat{columns}")
    } else {
        format!("{prefix}mat{columns}x{rows}")
    }
}

struct Resource<'a> {
    scalar: &'static str,
    shape: &'a str,
    shadow: bool,
}

fn resource<'a>(ty: &'a str, kind: &str) -> Option<Resource<'a>> {
    let (scalar, ty) = if ty.starts_with(kind) {
        ("float", ty)
    } else if let Some(ty) = ty.strip_prefix('i') {
        ("int", ty)
    } else if let Some(ty) = ty.strip_prefix('u') {
        ("uint", ty)
    } else {
        ("float", ty)
    };
    let suffix = ty.strip_prefix(kind)?;
    let shadow = suffix.ends_with("Shadow");
    let shape = suffix.strip_suffix("Shadow").unwrap_or(suffix);
    if shadow
        && (scalar != "float"
            || kind != "sampler"
            || !matches!(
                shape,
                "1D" | "2D" | "Cube" | "2DRect" | "1DArray" | "2DArray" | "CubeArray"
            ))
    {
        return None;
    }
    if kind == "subpassInput" {
        if !matches!(shape, "" | "MS") {
            return None;
        }
    } else if resource_dimensions(shape).is_none() {
        return None;
    }
    Some(Resource {
        scalar,
        shape,
        shadow,
    })
}

fn resource_dimensions(shape: &str) -> Option<usize> {
    match shape {
        "1D" | "Buffer" => Some(1),
        "2D" | "Cube" | "2DRect" | "1DArray" | "2DMS" => Some(2),
        "3D" | "2DArray" | "CubeArray" | "2DMSArray" => Some(3),
        _ => None,
    }
}

fn resource_return_type(name: &str, ty: &str, count: usize) -> Option<String> {
    if name == "subpassLoad" {
        let resource = resource(ty, "subpassInput")?;
        let expected = if resource.shape == "MS" { 2 } else { 1 };
        return (count == expected).then(|| with_scalar(resource.scalar, 4));
    }
    if name.starts_with("image") {
        let resource = resource(ty, "image")?;
        let operands = if resource.shape.contains("MS") { 3 } else { 2 };
        return match name {
            "imageSize" if count == 1 => {
                Some(with_scalar("int", resource_dimensions(resource.shape)?))
            }
            "imageSamples" if count == 1 && resource.shape.contains("MS") => Some("int".into()),
            "imageLoad" if count == operands => Some(with_scalar(resource.scalar, 4)),
            "imageStore" if count == operands + 1 => Some("void".into()),
            "imageAtomicAdd"
            | "imageAtomicMin"
            | "imageAtomicMax"
            | "imageAtomicAnd"
            | "imageAtomicOr"
            | "imageAtomicXor"
            | "imageAtomicExchange"
            | "imageAtomicCompSwap"
                if count == operands + if name == "imageAtomicCompSwap" { 2 } else { 1 }
                    && (resource.scalar != "float" || name == "imageAtomicExchange") =>
            {
                Some(resource.scalar.into())
            }
            _ => None,
        };
    }
    let resource = resource(ty, "sampler")?;
    let mipmapped = !matches!(resource.shape, "2DRect" | "Buffer" | "2DMS" | "2DMSArray");
    match name {
        "textureSize" if count == if mipmapped { 2 } else { 1 } => {
            Some(with_scalar("int", resource_dimensions(resource.shape)?))
        }
        "textureQueryLod" if count == 2 && mipmapped => Some("vec2".into()),
        "textureQueryLevels" if count == 1 && mipmapped => Some("int".into()),
        "textureSamples" if count == 1 && resource.shape.contains("MS") => Some("int".into()),
        "textureGather" | "textureGatherOffset" | "textureGatherOffsets" => {
            let offset = name != "textureGather";
            let base = if offset { 3 } else { 2 };
            let shape_ok = matches!(resource.shape, "2D" | "2DRect" | "2DArray")
                || (!offset && matches!(resource.shape, "Cube" | "CubeArray"));
            let count_ok = if resource.shadow {
                count == base + 1
            } else {
                (base..=base + 1).contains(&count)
            };
            (shape_ok && count_ok).then(|| with_scalar(resource.scalar, 4))
        }
        "texelFetch" | "texelFetchOffset" => {
            let offset = name == "texelFetchOffset";
            if resource.shadow
                || matches!(resource.shape, "Cube" | "CubeArray")
                || (offset && matches!(resource.shape, "Buffer" | "2DMS" | "2DMSArray"))
            {
                return None;
            }
            let base = if matches!(resource.shape, "Buffer" | "2DRect") {
                2
            } else {
                3
            };
            (count == base + usize::from(offset)).then(|| with_scalar(resource.scalar, 4))
        }
        "texture"
        | "textureProj"
        | "textureOffset"
        | "textureProjOffset"
        | "textureLod"
        | "textureProjLod"
        | "textureLodOffset"
        | "textureProjLodOffset"
        | "textureGrad"
        | "textureProjGrad"
        | "textureGradOffset"
        | "textureProjGradOffset" => {
            if matches!(resource.shape, "Buffer" | "2DMS" | "2DMSArray")
                || (name.contains("Proj")
                    && !matches!(resource.shape, "1D" | "2D" | "3D" | "2DRect"))
                || (name.contains("Offset") && matches!(resource.shape, "Cube" | "CubeArray"))
                || (name.contains("Lod") && !mipmapped)
                || (resource.shadow && resource.shape == "CubeArray" && name != "texture")
                || (resource.shadow
                    && name.contains("Lod")
                    && !matches!(resource.shape, "1D" | "2D" | "1DArray"))
            {
                return None;
            }
            let extra = usize::from(name.contains("Offset"))
                + if name.contains("Grad") {
                    2
                } else {
                    usize::from(name.contains("Lod"))
                };
            let base = 2 + extra + usize::from(resource.shadow && resource.shape == "CubeArray");
            let bias = !name.contains("Lod")
                && !name.contains("Grad")
                && mipmapped
                && !(resource.shadow && matches!(resource.shape, "2DArray" | "CubeArray"));
            (count == base || (bias && count == base + 1))
                .then(|| with_scalar(resource.scalar, if resource.shadow { 1 } else { 4 }))
        }
        _ => None,
    }
}
