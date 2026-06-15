use serde_json::json;
use ubu_core::core::{MootReasonCode, Task, TaskStatus};

#[test]
fn persisted_task_status_accepts_only_canonical_lifecycle_values() {
    let cases = [
        ("active", TaskStatus::Active, None),
        ("completed", TaskStatus::Completed, None),
        ("failed", TaskStatus::Failed, None),
        ("moot", TaskStatus::Moot, Some(MootReasonCode::Duplicate)),
    ];

    for (wire_status, expected, moot_reason_code) in cases {
        let mut task = json!({
            "id": "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f",
            "title": "Canonical lifecycle status",
            "status": wire_status,
            "provenance": {
                "created_at": "2026-06-10T14:30:00Z",
                "authority_source": "user"
            }
        });
        if let Some(moot_reason_code) = moot_reason_code {
            task["moot_reason_code"] =
                serde_json::to_value(moot_reason_code).expect("serialize moot reason code");
        }

        let task: Task = serde_json::from_value(task).expect("canonical status deserializes");

        assert_eq!(task.status, expected);
        assert_eq!(
            serde_json::to_value(task.status).expect("serialize task status"),
            json!(wire_status)
        );
    }
}

#[test]
fn readiness_terms_are_not_persisted_task_status_values() {
    for status in ["ready", "blocked"] {
        let task = json!({
            "id": "task_018f3c8e9b2a7c4d8f1e2a3b4c5d6e7f",
            "title": "Computed readiness is not lifecycle status",
            "status": status,
            "provenance": {
                "created_at": "2026-06-10T14:30:00Z",
                "authority_source": "user"
            }
        });

        assert!(
            serde_json::from_value::<Task>(task).is_err(),
            "{status} must remain computed readiness, not serialized Task.status"
        );
    }
}
