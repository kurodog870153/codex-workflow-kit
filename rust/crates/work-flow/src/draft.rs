//! Task draft command flows.

use serde_json::{Value, json};
use work_feature::error::{ExitCode, WorkError};

pub fn read(
    task_id: Option<&str>,
    read_index: impl FnOnce() -> Result<Value, WorkError>,
    read_draft: impl FnOnce(&Value, &str) -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    let index = read_index()?;
    if let Some(task_id) = task_id {
        read_draft(&index, task_id)
    } else {
        Ok(index)
    }
}

pub fn status(read: impl FnOnce() -> Result<Value, WorkError>) -> Result<Value, WorkError> {
    read()
}

pub fn semantic_prepare(
    prepare: impl FnOnce() -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    prepare()
}

pub fn list_update(update: impl FnOnce() -> Result<Value, WorkError>) -> Result<Value, WorkError> {
    update()
}

pub fn source_update(
    update: impl FnOnce() -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    update()
}

pub fn with_source_selection(
    explicit: bool,
    has_references: bool,
    action: impl FnOnce() -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    if !explicit && has_references {
        return Err(WorkError::new(
            ExitCode::CliUsage,
            "draft_selection_incomplete",
            "--reference requires --general-only or --instruction-path.",
            json!({}),
        ));
    }
    action()
}

pub fn save(
    operation: &str,
    request: &Value,
    revision: u64,
    save: impl FnOnce(&Value, u64, Option<&Value>) -> Result<Value, WorkError>,
    recover: impl FnOnce(&Value, u64, Option<&Value>) -> Result<Value, WorkError>,
) -> Result<Value, WorkError> {
    if operation == "draft-init" {
        return save(request, 0, None);
    }
    let (index, draft) = if operation == "draft-recover" && revision == 0 {
        (request, None)
    } else {
        let object = request.as_object().ok_or_else(|| {
            WorkError::new(
                ExitCode::Contract,
                "expected_object",
                "A JSON object is required.",
                json!({"location":"draft_save"}),
            )
        })?;
        let missing = ["index", "draft"]
            .into_iter()
            .filter(|field| !object.contains_key(*field))
            .collect::<Vec<_>>();
        let unknown = object
            .keys()
            .filter(|field| !["index", "draft"].contains(&field.as_str()))
            .collect::<Vec<_>>();
        if !missing.is_empty() || !unknown.is_empty() {
            return Err(WorkError::new(
                ExitCode::Contract,
                "invalid_object_fields",
                "The JSON object has missing or unknown fields.",
                json!({"location":"draft_save","missing":missing,"unknown":unknown}),
            ));
        }
        if !request["draft"].is_object() {
            return Err(WorkError::new(
                ExitCode::Contract,
                "expected_object",
                "draft must be a JSON object.",
                json!({}),
            ));
        }
        (&request["index"], Some(&request["draft"]))
    };
    if operation == "draft-recover" {
        recover(index, revision, draft)
    } else {
        save(index, revision, draft)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_save_stops_before_any_write_port() {
        let error = save(
            "draft-save",
            &json!({"index":{}}),
            1,
            |_, _, _| panic!("save port must not run"),
            |_, _, _| panic!("recover port must not run"),
        )
        .unwrap_err();
        assert_eq!(error.reason_code, "invalid_object_fields");
    }
}
