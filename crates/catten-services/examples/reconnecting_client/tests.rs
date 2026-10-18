use super::*;

fn reply(result: i64) -> CallResult {
    CallResult {
        result,
        connection: None,
        memory: None,
    }
}

#[test]
fn only_endpoint_loss_requests_reconnection() {
    assert!(matches!(
        admission_error(IpcError::Status(ipc_status::ENDPOINT_CLOSED)),
        Failure::ServiceRestarted
    ));
    assert!(matches!(decode(reply(IPC_REPLY_ENDPOINT_CLOSED)), Err(Failure::ServiceRestarted)));
    for status in [
        ipc_status::UNKNOWN_CAPABILITY,
        ipc_status::QUEUE_FULL,
        ipc_status::RESOURCE_LIMIT,
        ipc_status::PERMISSION_DENIED,
    ] {
        assert!(
            matches!(admission_error(IpcError::Status(status)), Failure::Ipc(IpcError::Status(s)) if s == status)
        );
    }
}

#[test]
fn cancellation_has_no_replay_authority() {
    assert!(matches!(decode(reply(IPC_REPLY_CANCELLED)), Err(Failure::OutcomeUnknown)));
}

#[test]
fn business_reply_requires_the_current_protocol_value() {
    assert_eq!(decode(reply(deploy::GREET_VALUE as i64)).unwrap(), deploy::GREET_VALUE);
    assert!(matches!(decode(reply(0)), Err(Failure::InvalidReply)));
    assert!(matches!(decode(reply(-99)), Err(Failure::Rejected(-99))));
}
