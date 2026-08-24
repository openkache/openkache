//! Server adapter for generated codec descriptors.
//!
//! Wire-value validation and container traversal live in
//! [`crate::openkache_protocol::codec`]. This module only resolves generated codec
//! identifiers and checks that the modeled registry is complete.

#![allow(dead_code)]

use super::operation_contract as contract;

/// Returns the generated descriptor for one codec identifier.
pub(crate) fn descriptor(name: &str) -> Option<contract::WireCodecDescriptor> {
    contract::WIRE_CODEC_DESCRIPTORS
        .iter()
        .copied()
        .find(|descriptor| descriptor.name == name)
}

/// Returns whether the server has a semantic adapter for one model codec.
pub(super) fn supports(name: &str) -> bool {
    descriptor(name).is_some()
        && contract::WIRE_CODEC_NAMES
            .iter()
            .any(|candidate| const_str_eq(candidate, name))
}

/// Verifies every generated request/response codec before the server binds.
pub(super) fn validate_contract_codecs() -> Result<(), &'static str> {
    if contract::WIRE_CODEC_NAMES
        .iter()
        .enumerate()
        .any(|(index, name)| {
            contract::WIRE_CODEC_NAMES[..index]
                .iter()
                .any(|candidate| candidate == name)
        })
    {
        return Err("generated wire codec registry contains duplicate names");
    }
    if contract::WIRE_CODEC_DESCRIPTORS.len() != contract::WIRE_CODEC_NAMES.len() {
        return Err("generated wire codec descriptor registry is not an exact set");
    }
    for name in contract::WIRE_CODEC_NAMES {
        if descriptor(name).is_none() {
            return Err("generated wire codec has no server descriptor");
        }
    }
    if contract::WIRE_CODEC_DESCRIPTORS.iter().any(|descriptor| {
        !contract::WIRE_CODEC_NAMES
            .iter()
            .any(|name| *name == descriptor.name)
    }) {
        return Err("server codec descriptor is not present in the generated registry");
    }
    if contract::WIRE_CODEC_DESCRIPTORS
        .iter()
        .enumerate()
        .any(|(index, descriptor)| {
            contract::WIRE_CODEC_DESCRIPTORS[..index]
                .iter()
                .any(|candidate| candidate.name == descriptor.name)
        })
    {
        return Err("server codec descriptor registry contains duplicate names");
    }
    for descriptor in contract::WIRE_CODEC_DESCRIPTORS {
        // Cardinality describes payload multiplicity, while `container` and
        // `recursive` describe whether the codec has traversable child
        // shapes. A packed repeated scalar is therefore valid without being
        // a recursive list container.
        let expected_container = matches!(
            descriptor.kind,
            contract::WireCodecKind::List
                | contract::WireCodecKind::Map
                | contract::WireCodecKind::Union
        );
        if descriptor.container != expected_container || descriptor.recursive != expected_container
        {
            return Err("generated wire codec shape metadata disagrees with its kind");
        }
        let expected_cardinality = match descriptor.kind {
            contract::WireCodecKind::List => Some(contract::WireCodecCardinality::Repeated),
            contract::WireCodecKind::Map => Some(contract::WireCodecCardinality::Associative),
            contract::WireCodecKind::Union => Some(contract::WireCodecCardinality::Tagged),
            _ => None,
        };
        if expected_cardinality.is_some_and(|expected| descriptor.cardinality != expected)
            || (!descriptor.container
                && matches!(
                    descriptor.cardinality,
                    contract::WireCodecCardinality::Associative
                        | contract::WireCodecCardinality::Tagged
                ))
        {
            return Err("generated wire codec cardinality metadata disagrees with its kind");
        }
        match descriptor.width {
            contract::WireCodecWidth::Fixed(width)
                if descriptor.min_width != width || descriptor.max_width != width =>
            {
                return Err("fixed codec width metadata is inconsistent");
            }
            contract::WireCodecWidth::Variable if descriptor.max_width < descriptor.min_width => {
                return Err("variable codec width metadata is inconsistent");
            }
            _ => {}
        }
        if descriptor.container
            && descriptor.length_encoding == contract::WireCodecLengthEncoding::None
        {
            return Err("container codec is missing a length encoding");
        }
    }
    for entry in contract::operation_registry() {
        let contract = entry.wire;
        for field in contract.request.fields {
            if field.nested_codecs.len() != field.nested_enum_values.len()
                || field.nested_codecs.len() != field.nested_union_tags.len()
                || (!field.nested_widths.is_empty()
                    && field.nested_codecs.len() != field.nested_widths.len())
            {
                return Err("operation contract nested codec metadata is misaligned");
            }
            for codec in field.codecs {
                if !supports(codec) {
                    return Err("operation contract requires an unsupported request codec");
                }
            }
            for codec in field.nested_codecs {
                if !supports(codec) {
                    return Err("operation contract requires an unsupported nested request codec");
                }
            }
        }
        for field in contract.response.fields {
            if field.nested_codecs.len() != field.nested_enum_values.len()
                || field.nested_codecs.len() != field.nested_union_tags.len()
                || (!field.nested_widths.is_empty()
                    && field.nested_codecs.len() != field.nested_widths.len())
            {
                return Err("operation contract nested codec metadata is misaligned");
            }
            for codec in field.codecs {
                if !supports(codec) {
                    return Err("operation contract requires an unsupported response codec");
                }
            }
            for codec in field.nested_codecs {
                if !supports(codec) {
                    return Err("operation contract requires an unsupported nested response codec");
                }
            }
        }
    }
    Ok(())
}

fn const_str_eq(left: &str, right: &str) -> bool {
    left.as_bytes() == right.as_bytes()
}
