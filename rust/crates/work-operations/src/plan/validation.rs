//! Pure structural and reference validation for confirmed Plans.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanIssue {
    pub reason_code: &'static str,
    pub message: &'static str,
    pub details: Value,
}

fn issue(reason_code: &'static str, message: &'static str, details: Value) -> PlanIssue {
    PlanIssue {
        reason_code,
        message,
        details,
    }
}

fn object<'a>(
    value: &'a Value,
    location: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<&'a serde_json::Map<String, Value>, PlanIssue> {
    let Some(object) = value.as_object() else {
        return Err(issue(
            "expected_object",
            "A JSON object is required.",
            json!({"location": location}),
        ));
    };
    let mut missing: Vec<_> = required
        .iter()
        .filter(|field| !object.contains_key(**field))
        .copied()
        .collect();
    missing.sort_unstable();
    let unknown: Vec<_> = object
        .keys()
        .filter(|field| !required.contains(&field.as_str()) && !optional.contains(&field.as_str()))
        .cloned()
        .collect();
    if !missing.is_empty() || !unknown.is_empty() {
        return Err(issue(
            "invalid_object_fields",
            "The JSON object has missing or unknown fields.",
            json!({"location": location, "missing": missing, "unknown": unknown}),
        ));
    }
    Ok(object)
}

fn text<'a>(value: &'a Value, location: &str) -> Result<&'a str, PlanIssue> {
    value
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            issue(
                "empty_text_value",
                "A non-empty string is required.",
                json!({"location": location}),
            )
        })
}

fn string_array(
    value: &Value,
    location: &str,
    allow_empty: bool,
) -> Result<Vec<String>, PlanIssue> {
    let Some(values) = value
        .as_array()
        .filter(|values| allow_empty || !values.is_empty())
    else {
        return Err(issue(
            "invalid_string_array",
            "A non-empty string array is required.",
            json!({"location": location}),
        ));
    };
    let result: Vec<_> = values
        .iter()
        .map(|value| text(value, &format!("{location}[]")).map(str::to_owned))
        .collect::<Result<_, _>>()?;
    if result.iter().collect::<BTreeSet<_>>().len() != result.len() {
        return Err(issue(
            "duplicate_array_value",
            "Array values must be unique.",
            json!({"location": location}),
        ));
    }
    Ok(result)
}

fn prefix(key: &str) -> &'static str {
    match key {
        "goals" => "GOAL",
        "scope" => "SCOPE",
        "constraints" => "CONSTRAINT",
        "dependencies" => "DEPENDENCY",
        "risks" => "RISK",
        "milestones" => "MILESTONE",
        "deliverables" => "DELIVERABLE",
        "acceptance_criteria" => "ACCEPTANCE",
        "decisions" => "PLAN-DECISION",
        "changes" => "PLAN-CHANGE",
        _ => unreachable!(),
    }
}

fn numeric_id(value: &str, expected: &str) -> Option<usize> {
    let tail = value.strip_prefix(expected)?.strip_prefix('-')?;
    (tail.len() == 3 && tail.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| tail.parse().ok())
        .flatten()
}

fn items<'a>(
    plan: &'a serde_json::Map<String, Value>,
    key: &str,
    required: &[&str],
    optional: &[&str],
) -> Result<Vec<&'a serde_json::Map<String, Value>>, PlanIssue> {
    let values = plan[key]
        .as_array()
        .filter(|values| !values.is_empty())
        .ok_or_else(|| {
            issue(
                "invalid_item_array",
                "A present Plan item array must be non-empty.",
                json!({"location": key}),
            )
        })?;
    let mut result = Vec::new();
    let mut previous = 0;
    for (index, item) in values.iter().enumerate() {
        let mut fields = vec!["id"];
        fields.extend_from_slice(required);
        let item = object(item, &format!("{key}[{index}]"), &fields, optional)?;
        let id = text(&item["id"], &format!("{key}[{index}].id"))?;
        let Some(number) = numeric_id(id, prefix(key)).filter(|number| *number > previous) else {
            return Err(issue(
                "invalid_or_unsorted_id",
                "Plan item IDs must use the expected prefix and ascending numeric order.",
                json!({"location": key, "id": id}),
            ));
        };
        previous = number;
        result.push(item);
    }
    Ok(result)
}

fn references(
    value: &Value,
    expected: &str,
    known: &BTreeSet<String>,
    location: &str,
) -> Result<Vec<String>, PlanIssue> {
    let references = string_array(value, location, false)?;
    let mut numbers = Vec::new();
    for id in &references {
        let Some(number) = numeric_id(id, expected).filter(|_| known.contains(id)) else {
            return Err(issue(
                "invalid_reference",
                "A referenced Plan ID does not exist or has the wrong type.",
                json!({"location": location, "id": id}),
            ));
        };
        numbers.push(number);
    }
    if numbers.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err(issue(
            "unsorted_references",
            "Referenced IDs must use ascending numeric order.",
            json!({"location": location}),
        ));
    }
    Ok(references)
}

pub fn validate_plan_structure(value: &Value) -> Result<usize, PlanIssue> {
    let required = [
        "schema",
        "requirement_id",
        "status",
        "title",
        "summary",
        "artifacts",
        "hierarchy_selection",
        "work_instruction_selection",
        "skill_selection",
        "goals",
        "scope",
        "deliverables",
        "acceptance_criteria",
    ];
    let optional = [
        "constraints",
        "dependencies",
        "risks",
        "milestones",
        "decisions",
        "changes",
    ];
    let plan = object(value, "plan", &required, &optional)?;
    if plan["schema"] != "work-plan/v1" {
        return Err(issue(
            "invalid_plan_schema",
            "Invalid Plan schema.",
            json!({}),
        ));
    }
    if plan["status"] != "confirmed" {
        return Err(issue(
            "invalid_plan_status",
            "A formal Plan status must be confirmed.",
            json!({}),
        ));
    }
    let title = text(&plan["title"], "title")?;
    if title.contains(['\n', '\r']) {
        return Err(issue(
            "invalid_plan_title",
            "The Plan title must fit on one line.",
            json!({}),
        ));
    }
    text(&plan["summary"], "summary")?;
    let fields: [(&str, &[&str], &[&str]); 10] = [
        ("goals", &["statement"], &[]),
        ("scope", &["kind", "statement"], &["goal_ids"]),
        ("constraints", &["statement", "applies_to"], &[]),
        ("dependencies", &["statement", "applies_to"], &[]),
        (
            "risks",
            &["condition", "impact", "mitigation", "applies_to"],
            &[],
        ),
        ("milestones", &["statement", "deliverable_ids"], &[]),
        (
            "deliverables",
            &["statement", "goal_ids", "acceptance_ids"],
            &[],
        ),
        (
            "acceptance_criteria",
            &["statement", "deliverable_ids"],
            &[],
        ),
        ("decisions", &["statement", "rationale", "applies_to"], &[]),
        (
            "changes",
            &[
                "date",
                "location",
                "before",
                "after",
                "reason",
                "affected_ids",
            ],
            &[],
        ),
    ];
    let mut groups = BTreeMap::new();
    let mut known = BTreeSet::new();
    for (key, required, optional) in fields {
        if plan.contains_key(key) {
            let group = items(plan, key, required, optional)?;
            for item in &group {
                let id = item["id"].as_str().unwrap();
                if !known.insert(id.to_owned()) {
                    return Err(issue(
                        "duplicate_plan_id",
                        "Plan IDs must be globally unique.",
                        json!({"id": id}),
                    ));
                }
            }
            groups.insert(key, group);
        }
    }
    for item in &groups["goals"] {
        text(
            &item["statement"],
            &format!("{}.statement", item["id"].as_str().unwrap()),
        )?;
    }
    for item in &groups["scope"] {
        let id = item["id"].as_str().unwrap();
        let kind = item["kind"].as_str();
        if !matches!(kind, Some("in_scope" | "out_of_scope")) {
            return Err(issue(
                "invalid_scope_kind",
                "Invalid scope kind.",
                json!({}),
            ));
        }
        text(&item["statement"], &format!("{id}.statement"))?;
        if kind == Some("in_scope") && !item.contains_key("goal_ids") {
            return Err(issue(
                "in_scope_without_goals",
                "An in-scope item must reference at least one Goal.",
                json!({"id": id}),
            ));
        }
        if let Some(ids) = item.get("goal_ids") {
            references(ids, "GOAL", &known, &format!("{id}.goal_ids"))?;
        }
    }
    let mut referenced_goals = BTreeSet::new();
    let mut deliverable_acceptances = BTreeMap::new();
    let mut acceptance_deliverables = BTreeMap::new();
    for item in &groups["deliverables"] {
        let id = item["id"].as_str().unwrap();
        text(&item["statement"], &format!("{id}.statement"))?;
        referenced_goals.extend(references(
            &item["goal_ids"],
            "GOAL",
            &known,
            &format!("{id}.goal_ids"),
        )?);
        deliverable_acceptances.insert(
            id.to_owned(),
            references(
                &item["acceptance_ids"],
                "ACCEPTANCE",
                &known,
                &format!("{id}.acceptance_ids"),
            )?,
        );
    }
    for item in &groups["acceptance_criteria"] {
        let id = item["id"].as_str().unwrap();
        text(&item["statement"], &format!("{id}.statement"))?;
        acceptance_deliverables.insert(
            id.to_owned(),
            references(
                &item["deliverable_ids"],
                "DELIVERABLE",
                &known,
                &format!("{id}.deliverable_ids"),
            )?,
        );
    }
    let goal_ids: BTreeSet<_> = groups["goals"]
        .iter()
        .map(|item| item["id"].as_str().unwrap().to_owned())
        .collect();
    if referenced_goals != goal_ids {
        return Err(issue(
            "untraced_goal",
            "Every Goal must be referenced by at least one Deliverable.",
            json!({"untraced_ids": goal_ids.difference(&referenced_goals).collect::<Vec<_>>()}),
        ));
    }
    for (deliverable, acceptances) in &deliverable_acceptances {
        for acceptance in acceptances {
            if !acceptance_deliverables[acceptance].contains(deliverable) {
                return Err(issue(
                    "deliverable_acceptance_mismatch",
                    "Deliverable and Acceptance references must be bidirectional.",
                    json!({"deliverable_id": deliverable, "acceptance_id": acceptance}),
                ));
            }
        }
    }
    for (acceptance, deliverables) in &acceptance_deliverables {
        for deliverable in deliverables {
            if !deliverable_acceptances[deliverable].contains(acceptance) {
                return Err(issue(
                    "deliverable_acceptance_mismatch",
                    "Deliverable and Acceptance references must be bidirectional.",
                    json!({"deliverable_id": deliverable, "acceptance_id": acceptance}),
                ));
            }
        }
    }
    for key in ["constraints", "dependencies"] {
        if let Some(group) = groups.get(key) {
            for item in group {
                let id = item["id"].as_str().unwrap();
                text(&item["statement"], &format!("{id}.statement"))?;
                applies_to(&item["applies_to"], id, &known)?;
            }
        }
    }
    if let Some(group) = groups.get("risks") {
        for item in group {
            let id = item["id"].as_str().unwrap();
            for field in ["condition", "impact", "mitigation"] {
                text(&item[field], &format!("{id}.{field}"))?;
            }
            applies_to(&item["applies_to"], id, &known)?;
        }
    }
    if let Some(group) = groups.get("milestones") {
        for item in group {
            let id = item["id"].as_str().unwrap();
            text(&item["statement"], &format!("{id}.statement"))?;
            references(
                &item["deliverable_ids"],
                "DELIVERABLE",
                &known,
                &format!("{id}.deliverable_ids"),
            )?;
        }
    }
    if let Some(group) = groups.get("decisions") {
        for item in group {
            let id = item["id"].as_str().unwrap();
            text(&item["statement"], &format!("{id}.statement"))?;
            text(&item["rationale"], &format!("{id}.rationale"))?;
            applies_to(&item["applies_to"], id, &known)?;
        }
    }
    if let Some(group) = groups.get("changes") {
        for item in group {
            let id = item["id"].as_str().unwrap();
            for field in ["location", "before", "after", "reason"] {
                text(&item[field], &format!("{id}.{field}"))?;
            }
            let date = text(&item["date"], &format!("{id}.date"))?;
            if !valid_date(date) {
                return Err(issue(
                    "invalid_change_date",
                    "A Plan change date must use YYYY-MM-DD.",
                    json!({"id": id}),
                ));
            }
            for reference in
                string_array(&item["affected_ids"], &format!("{id}.affected_ids"), false)?
            {
                if !known.contains(&reference) {
                    return Err(issue(
                        "invalid_reference",
                        "A Plan change references an unknown affected ID.",
                        json!({"id": id}),
                    ));
                }
            }
        }
    }
    let _: work_model::plan::PlanArtifact =
        serde_json::from_value(value.clone()).expect("validated Plan matches its model");
    Ok(known.len())
}

fn applies_to(value: &Value, owner: &str, known: &BTreeSet<String>) -> Result<(), PlanIssue> {
    for reference in string_array(value, &format!("{owner}.applies_to"), false)? {
        if reference != "PLAN" && !known.contains(&reference) {
            return Err(issue(
                "invalid_reference",
                "An applies_to reference is unknown.",
                json!({"id": owner, "reference": reference}),
            ));
        }
    }
    Ok(())
}

fn valid_date(value: &str) -> bool {
    let parts: Vec<_> = value.split('-').collect();
    if parts.len() != 3
        || parts[0].len() != 4
        || parts[1].len() != 2
        || parts[2].len() != 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'-')
    {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        parts[0].parse::<i32>(),
        parts[1].parse::<u32>(),
        parts[2].parse::<u32>(),
    ) else {
        return false;
    };
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    };
    day >= 1 && day <= days
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_fixture_has_valid_structure() {
        let fixtures: Value = serde_json::from_str(include_str!(
            "../../../../crates/work-operations/fixtures.json"
        ))
        .unwrap();
        let mut plan = fixtures["plan_artifact"]["input"].clone();
        plan["goals"] = json!([{"id": "GOAL-001", "statement": "Goal."}]);
        plan["scope"] = json!([{"id": "SCOPE-001", "kind": "in_scope", "statement": "Scope.", "goal_ids": ["GOAL-001"]}]);
        plan["deliverables"] = json!([{"id": "DELIVERABLE-001", "statement": "Delivery.", "goal_ids": ["GOAL-001"], "acceptance_ids": ["ACCEPTANCE-001"]}]);
        plan["acceptance_criteria"] = json!([{"id": "ACCEPTANCE-001", "statement": "Accepted.", "deliverable_ids": ["DELIVERABLE-001"]}]);
        assert_eq!(validate_plan_structure(&plan).unwrap(), 4);
        plan["deliverables"][0]["acceptance_ids"] = json!(["ACCEPTANCE-999"]);
        assert_eq!(
            validate_plan_structure(&plan).unwrap_err().reason_code,
            "invalid_reference"
        );
    }
}
