//! Operation-neutral projection of admitted request frames.
//!
//! Generated wire metadata maps one owned frame into numeric modeled fields.
//! This boundary does not classify API families or interpret field semantics.

use crate::openkache_protocol::{
    OPCODE_BYTES, Opcode, OwnedRange, ProtocolError, RequestFieldProjection, project_request_frame,
};

use super::operation_contract as contract;
use super::operation_handlers::{OperationFieldRecord, OperationFieldStorage, OperationInputView};

/// Projects one complete owned frame into its generated numeric field view.
///
/// # Errors
///
/// Returns a protocol error when the frame is malformed or its generated
/// layout and modeled field plan disagree.
pub(super) fn project_owned_request(frame: Vec<u8>) -> Result<OperationInputView, ProtocolError> {
    let opcode_byte = frame.first().copied().ok_or(ProtocolError::FrameTooShort {
        expected: OPCODE_BYTES,
        actual: frame.len(),
    })?;
    let opcode = Opcode::try_from(opcode_byte)?;
    let operation_id = contract::operation_id_for_opcode(opcode);
    let plan = contract::operation_wire_spec_for_id(operation_id)
        .request
        .fields;
    let layout = contract::wire_request_layout_for_id(operation_id);
    if layout.field_count != plan.len() {
        return Err(ProtocolError::InvalidFieldSequence(
            "request layout field count does not match modeled fields",
        ));
    }
    if plan.len() > contract::MAX_OPERATION_REQUEST_FIELDS {
        return Err(ProtocolError::InvalidFieldSequence(
            "request field plan exceeds generated projection storage",
        ));
    }

    let mut projections = [RequestFieldProjection::Missing; contract::MAX_OPERATION_REQUEST_FIELDS];
    let projected_header = project_request_frame(&frame, layout, &mut projections)?;
    if projected_header.opcode() != opcode {
        return Err(ProtocolError::InvalidFieldSequence(
            "projected request opcode does not match frame opcode",
        ));
    }

    let fields = plan
        .iter()
        .zip(projections[..plan.len()].iter().copied())
        .map(|(field_plan, projection)| {
            let value = match projection {
                RequestFieldProjection::Missing => None,
                RequestFieldProjection::Borrowed { start, end } => {
                    Some(OperationFieldStorage::OwnerRange { start, end })
                }
                RequestFieldProjection::Inline(bytes) => {
                    Some(OperationFieldStorage::InlineBytes { bytes, len: 8 })
                }
                RequestFieldProjection::Static(bytes) => {
                    Some(OperationFieldStorage::StaticBytes(bytes))
                }
            };
            Some(OperationFieldRecord {
                plan: field_plan,
                value,
            })
        });
    let input = OperationInputView::from_populated_projection(
        operation_id,
        projected_header.request_id(),
        OwnedRange::whole(frame),
        fields,
    );
    input
        .validate_populated_fields()
        .map_err(ProtocolError::InvalidFieldSequence)?;
    Ok(input)
}
