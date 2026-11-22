//! Validation gates (§11.0 `config validate`, §20.1 `schema validate`).
//!
//! Structural validation runs without a JSON Schema engine: required fields,
//! ID patterns, enum memberships, and cross-file consistency — all rules
//! mirrored from `schemas/*.schema.json`, which remain the canonical source
//! (a mismatch here is a bug in this module, not in the schemas).
//! Full schema validation plugs in during Phase 2 with the `jsonschema` crate.

use std::path::Path;

use serde_json::Value;

use crate::config;
use crate::error::{Error, Result};
use crate::models::spec::Spec;
use crate::models::task::TaskPlan;

/// `ribozym config validate` — parse every config file and enforce the
/// registry invariants the doc states (§5.1 kadro A1-A12, §24 rules).
pub fn validate_config(root: &Path) -> Result<Vec<String>> {
    let mut problems: Vec<String> = Vec::new();

    let project = config::load_project_config(root)?;
    let agents = config::load_agents(root)?;
    let models = config::load_models(root)?;

    // Agent registry invariants (v2.46: contiguous A1..A12; v2.45 verify-hat etc.)
    let expected: Vec<String> = (1..=12).map(|i| format!("A{i}")).collect();
    let got: Vec<String> = agents.iter().map(|a| a.id.clone()).collect();
    if got != expected {
        problems.push(format!(
            "agents.toml: agent ids must be contiguous A1..A12, got {got:?}"
        ));
    }
    for a in &agents {
        if !config::agents::VALID_TIERS.contains(&a.model_tier.as_str()) {
            problems.push(format!(
                "agents.toml: {} model_tier '{}' invalid",
                a.id, a.model_tier
            ));
        }
        if !config::agents::VALID_TIERS.contains(&a.escalation_tier.as_str()) {
            problems.push(format!(
                "agents.toml: {} escalation_tier '{}' invalid",
                a.id, a.escalation_tier
            ));
        }
        let prompt = root.join(&a.prompt_file);
        if !prompt.is_file() {
            problems.push(format!(
                "agents.toml: {} prompt_file missing: {}",
                a.id, a.prompt_file
            ));
        }
    }
    let total_share: u32 = agents.iter().map(|a| a.budget_share_pct).sum();
    if total_share != 100 {
        problems.push(format!(
            "agents.toml: budget_share_pct sums to {total_share}, expected 100"
        ));
    }

    // Model registry: unmeasured models cannot be candidates (§10.3) — the
    // registry itself only demands ids and non-negative fields.
    for m in &models.models {
        if m.id.is_empty() {
            problems.push("models.toml: model entry with empty id".to_string());
        }
    }

    // Local grunt-work whitelist (§10.1, Q1): membership-only routing means
    // the list must be well-formed; empty = local disabled (legal).
    {
        let mut seen: Vec<&str> = Vec::new();
        for t in &models.local.eligible_task_types {
            if t.trim().is_empty() {
                problems
                    .push("models.toml: [local] eligible_task_types entry is empty".to_string());
            } else if t != t.trim() || t.split_whitespace().count() != 1 {
                problems.push(format!(
                    "models.toml: [local] task type '{t}' must be a single trimmed token"
                ));
            }
            if seen.contains(&t.as_str()) {
                problems.push(format!("models.toml: [local] duplicate task type '{t}'"));
            }
            seen.push(t);
        }
    }

    // Budget invariants (§8.2)
    if project.budget.soft_limit_pct >= project.budget.hard_limit_pct {
        problems.push(format!(
            "ribozym.toml: soft_limit_pct ({}) must be < hard_limit_pct ({})",
            project.budget.soft_limit_pct, project.budget.hard_limit_pct
        ));
    }
    // v2.47 B1 constant
    if project.errors.final_review_crisis_max == 0 {
        problems.push("ribozym.toml: final_review_crisis_max must be >= 1 (B1)".to_string());
    }

    Ok(problems)
}

/// `ribozym schema validate <file>` — lightweight structural pass over a run
/// document against its schema's required fields and ID patterns.
/// Returns the list of violations; empty = valid.
pub fn validate_document(schema_name: &str, doc: &Value) -> Vec<String> {
    let mut problems: Vec<String> = Vec::new();
    let obj = match doc.as_object() {
        Some(o) => o,
        None => {
            return vec![format!("{schema_name}: document is not a JSON object")];
        }
    };

    // Required-field lists mirrored from schemas/ (subset: mandatory keys).
    let required: &[&str] = match schema_name {
        "spec.schema.json" => &[
            "schema_version",
            "id",
            "version",
            "created_at",
            "created_by",
            "source",
            "title",
            "description",
            "mode",
            "language",
            "requirements",
            "acceptance_criteria",
            "budget",
            "autonomy_level",
            "approval_required",
            "compliance_status",
        ],
        "tasks.schema.json" => &[
            "schema_version",
            "spec_id",
            "created_at",
            "created_by",
            "revision",
            "tasks",
        ],
        "execution_result.schema.json" => &[
            "schema_version",
            "task_id",
            "agent_id",
            "started_at",
            "finished_at",
            "status",
            "summary",
        ],
        "department_report.schema.json" => &[
            "schema_version",
            "spec_id",
            "generated_at",
            "generated_by",
            "final_status",
            "summary",
            "cost",
        ],
        "compliance_check.schema.json" => &[
            "schema_version",
            "spec_id",
            "checked_by",
            "spec_match_pct",
            "requirements",
            "verdict",
        ],
        "event.schema.json" => &[
            "seq",
            "ts",
            "spec_id",
            "run_id",
            "event",
            "correlation_id",
            "schema_version",
        ],
        "archive_manifest.schema.json" => &["schema_version", "spec_id", "sections"],
        "web_query.schema.json" => &["id", "ask"],
        "web_finding.schema.json" => &["query_id", "claim", "sources"],
        "verification_score.schema.json" => &["query_id", "confidence", "layers"],
        _ => {
            return vec![format!("unknown schema: {schema_name}")];
        }
    };
    for key in required {
        if !obj.contains_key(*key) {
            problems.push(format!("{schema_name}: missing required field '{key}'"));
        }
    }

    // `status_summary` is A2's DECLARED snapshot at authoring time. The control
    // plane derives the truth from the DAG (Scheduler::status_summary), so a
    // declared `total` that contradicts the actual task count is a LIE, not
    // staleness — reject it here (only `total` is checkable: the schema has no
    // `blocked` field, so the per-status counts need not sum to it). This is
    // the "tahmin uydurmaz" rule applied to the plan's own self-report.
    if schema_name == "tasks.schema.json" {
        if let (Some(tasks), Some(summary)) = (obj.get("tasks"), obj.get("status_summary")) {
            let n = tasks.as_array().map_or(0, |a| a.len());
            let total = summary.get("total").and_then(Value::as_u64).unwrap_or(0);
            if total > 0 && total as usize != n {
                problems.push(format!(
                    "STATUS_SUMMARY_STALE: declared total={total} but the plan lists {n} tasks"
                ));
            }
        }
    }

    // ID patterns (canonical: the schema `pattern` attributes).
    check_pattern(
        schema_name,
        obj,
        "spec_id",
        r"^spec-\d{8}-\d{3}$",
        &mut problems,
    );
    check_pattern(schema_name, obj, "id", r"^spec-\d{8}-\d{3}$", &mut problems);
    check_pattern(
        schema_name,
        obj,
        "run_id",
        r"^run-[a-z0-9]+$",
        &mut problems,
    );

    problems
}

fn check_pattern(
    schema_name: &str,
    obj: &serde_json::Map<String, Value>,
    field: &str,
    pattern: &str,
    problems: &mut Vec<String>,
) {
    if let Some(Value::String(s)) = obj.get(field) {
        if let Ok(re) = regex_lite(pattern) {
            if !re.is_match(s) {
                problems.push(format!(
                    "{schema_name}: field '{field}' = '{s}' does not match {pattern}"
                ));
            }
        }
    }
}

/// Tiny anchored-regex subset good enough for the ID patterns above
/// (no regex crate dependency in Phase 1).
fn regex_lite(pattern: &str) -> Result<SimplePattern> {
    SimplePattern::compile(pattern).map_err(Error::ConfigInvalid)
}

pub struct SimplePattern {
    kind: PatternKind,
}

enum PatternKind {
    SpecId,
    RunId,
}

impl SimplePattern {
    fn compile(pattern: &str) -> std::result::Result<Self, String> {
        match pattern {
            r"^spec-\d{8}-\d{3}$" => Ok(Self {
                kind: PatternKind::SpecId,
            }),
            r"^run-[a-z0-9]+$" => Ok(Self {
                kind: PatternKind::RunId,
            }),
            other => Err(format!("unsupported pattern in Phase 1: {other}")),
        }
    }

    fn is_match(&self, s: &str) -> bool {
        match self.kind {
            PatternKind::SpecId => {
                let b = s.as_bytes();
                b.len() == 17
                    && b.starts_with(b"spec-")
                    && b[5..13].iter().all(u8::is_ascii_digit)
                    && b[13] == b'-'
                    && b[14..].iter().all(u8::is_ascii_digit)
            }
            PatternKind::RunId => {
                let rest = s.strip_prefix("run-");
                match rest {
                    Some(r) if !r.is_empty() => r
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                    _ => false,
                }
            }
        }
    }
}

/// Full JSON Schema (draft 2020-12) validation against a `schemas/` file —
/// the canonical check for LLM output before it may become work (§3.1, §11.0).
/// `root` is the repo root containing `schemas/`.
pub fn validate_full(root: &std::path::Path, schema_name: &str, doc: &Value) -> Vec<String> {
    use jsonschema::JSONSchema;
    use std::sync::OnceLock;

    static CACHE: OnceLock<std::collections::HashMap<String, JSONSchema>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| {
        let mut m = std::collections::HashMap::new();
        let dir = root.join("schemas");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "json") {
                    if let (Some(name), Ok(text)) = (
                        path.file_name().map(|n| n.to_string_lossy().into_owned()),
                        std::fs::read_to_string(&path),
                    ) {
                        if let Ok(mut parsed) = serde_json::from_str::<Value>(&text) {
                            // Registry-local copies compile without $id: the
                            // repo's relative ids (e.g. "event.schema.json")
                            // are not resolvable URIs and only matter for
                            // cross-schema $ref over the wire, which we do
                            // not use.
                            if let Some(obj) = parsed.as_object_mut() {
                                obj.remove("$id");
                            }
                            if let Ok(v) = JSONSchema::compile(&parsed) {
                                m.insert(name, v);
                            }
                        }
                    }
                }
            }
        }
        m
    });

    match cache.get(schema_name) {
        Some(validator) => match validator.validate(doc) {
            Ok(()) => Vec::new(),
            Err(errors) => errors
                .map(|e| {
                    let path = e.instance_path.to_string();
                    if path.is_empty() {
                        format!("{schema_name}: {e}")
                    } else {
                        format!("{schema_name} at {path}: {e}")
                    }
                })
                .collect(),
        },
        None => vec![format!(
            "unknown or unloadable schema: {schema_name} (looked in {})",
            root.join("schemas").display()
        )],
    }
}

/// §6.4 traceability gate — the check that rejects the `TasksCreated`
/// transition when the task graph leaves any spec requirement uncovered
/// (`TASK_UNCOVERED_REQUIREMENT`). Pure over the two typed models; every
/// requirement id must appear in ≥1 task's `spec_requirement_ids`, and every
/// requirement id a task cites must exist in the spec (no phantom edges).
///
/// Returns the list of uncovered/unknown ids (empty ⇒ the graph is
/// traceable). This is the §6.4 cross-document gate; distinct from the
/// schema-level *orphan-task* rule (`tasks.schema.json` `minItems:1` on
/// `spec_requirement_ids`, checked by `validate_full`). A task citing nothing
/// is a schema error; a requirement cited by nobody is a traceability error —
/// both must hold before `TasksCreated` may fire.
pub fn check_traceability(spec: &Spec, plan: &TaskPlan) -> Vec<String> {
    let mut problems = Vec::new();

    let spec_reqs: std::collections::BTreeSet<&str> =
        spec.requirements.iter().map(|r| r.id.as_str()).collect();
    let covered: std::collections::BTreeSet<&str> = plan
        .tasks
        .iter()
        .flat_map(|t| t.spec_requirement_ids.iter().map(|s| s.as_str()))
        .collect();

    // Every spec requirement covered by ≥1 task.
    for r in spec_reqs.iter() {
        if !covered.contains(r) {
            problems.push(format!("TASK_UNCOVERED_REQUIREMENT: {r}"));
        }
    }
    // No task cites a requirement the spec never declared (dangling edge).
    for t in &plan.tasks {
        for cited in &t.spec_requirement_ids {
            if !spec_reqs.contains(cited.as_str()) {
                problems.push(format!(
                    "UNKNOWN_REQUIREMENT_REFERENCE: task {} cites {cited}, not in spec",
                    t.id
                ));
            }
        }
    }
    // Spec↔plan identity mismatch is a wiring bug, surfaced here too.
    if !plan.spec_id.is_empty() && plan.spec_id != spec.id {
        problems.push(format!(
            "SPEC_ID_MISMATCH: plan {p} vs spec {s}",
            p = plan.spec_id,
            s = spec.id
        ));
    }
    problems
}

/// §5.4 stage-1 completeness check — CODE checks the filled form BEFORE any
/// model call. A form missing the structure the §5.4C silent-guess ban
/// requires (sections, at least one must-have, a positive numeric budget,
/// a title) must park as NeedsUser, never be drafted by guessing.
/// Pure over the raw bytes; `<!-- -->` comments are documentation, not data.
pub fn check_form(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let body = strip_html_comments(&text);
    let mut problems: Vec<String> = Vec::new();

    // Title: first line must be a real one, not a bare template header.
    match body.lines().map(str::trim).find(|l| l.starts_with("# ")) {
        Some(l) if l.trim_end().ends_with(':') || l.contains('<') => {
            problems.push("title line is a template placeholder".into())
        }
        None => problems.push("no title line ('# ...') found".into()),
        _ => {}
    }

    // §5.4's sections, in template order; the analysis call reads them whole.
    const SECTIONS: [&str; 7] = [
        "## Summary",
        "## Goal & Users",
        "## Scope",
        "## Constraints",
        "## Budget",
        "## Delivery Shape",
        "## Success Criteria",
    ];
    for section in SECTIONS {
        match section_body(&body, section) {
            None => problems.push(format!("section '{section}' missing")),
            Some(b) if b.is_empty() => problems.push(format!("section '{section}' has no content")),
            _ => {}
        }
    }

    // must_have drives the MVP scope (§5.4: MVP = form must-items + accepted
    // suggestions) — an empty list means there is nothing to build.
    let scope = section_body(&body, "## Scope").unwrap_or_default();
    let items = list_items_after(&scope, "must_have:");
    if items.is_empty() {
        problems.push("scope: no must_have items (an empty MVP is not a project)".into());
    }

    // Budget: the spec locks a positive number (§5.6 — never invented later).
    let budget = section_body(&body, "## Budget").unwrap_or_default();
    let line = budget
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("- run_budget_usd:"));
    match line.and_then(|l| l.split(':').nth(1)).map(str::trim) {
        Some(v) => match v.parse::<f64>() {
            Ok(n) if n > 0.0 => {}
            _ => problems.push(format!(
                "budget: run_budget_usd is not a positive number ('{v}')"
            )),
        },
        None => problems.push("budget: run_budget_usd line missing".into()),
    }
    problems
}

/// Remove `<!-- ... -->` blocks (multi-line aware) — template guidance is not
/// an answer to the completeness check.
fn strip_html_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<!--") {
        out.push_str(&rest[..start]);
        match rest[start..].find("-->") {
            Some(end) => rest = &rest[start + end + 3..],
            None => return out, // unterminated comment: keep what precedes
        }
    }
    out.push_str(rest);
    out
}

/// Heading→body split of comment-stripped markdown: everything from each
/// `## Heading` line to the next one (or EOF), trimmed. A line-based scan so
/// no byte arithmetic can cut a UTF-8 character or index the wrong slice.
fn sections(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut cur: Option<(String, String)> = None;
    for line in text.lines() {
        let t = line.trim_end();
        if t.starts_with("## ") {
            if let Some(prev) = cur.take() {
                out.push(prev);
            }
            cur = Some((t.trim().to_string(), String::new()));
        } else if let Some((_, body)) = cur.as_mut() {
            body.push_str(t);
            body.push('\n');
        }
    }
    if let Some(prev) = cur {
        out.push(prev);
    }
    out
}

fn section_body(text: &str, section: &str) -> Option<String> {
    sections(text)
        .into_iter()
        .find(|(h, _)| h == section)
        .map(|(_, b)| b.trim().to_string())
}

/// Non-empty `- item` lines under `key:`. Items are nested (indented deeper
/// than the key); a sibling at the key's own indentation ends the list.
/// `[]` is the template's empty placeholder, not an item.
fn list_items_after(section: &str, key: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut in_list = false;
    let mut key_indent = 0usize;
    for line in section.lines() {
        let t = line.trim();
        let indent = line.len() - line.trim_start().len();
        if !in_list {
            let bare = t.strip_prefix("- ").unwrap_or(t);
            if bare.starts_with(key) {
                in_list = true;
                key_indent = indent;
            }
            continue;
        }
        if t.is_empty() {
            continue;
        }
        if indent <= key_indent && t.starts_with("- ") {
            break; // the next key at the same level: the must_have list ended
        }
        if let Some(item) = t.strip_prefix("- ") {
            let item = item.trim();
            if !item.is_empty() && item != "[]" {
                items.push(item.to_string());
            }
        }
    }
    items
}

#[cfg(test)]
mod form_tests {
    use super::check_form;

    #[test]
    fn the_golden_form_passes() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let form = std::fs::read(root.join("examples/spec_rest_api.md")).unwrap();
        assert_eq!(check_form(&form), Vec::<String>::new());
    }

    #[test]
    fn the_empty_template_fails_on_every_real_ground() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let form = std::fs::read(root.join("examples/spec_form.md")).unwrap();
        let problems = check_form(&form);
        assert!(problems.iter().any(|p| p.contains("title")), "{problems:?}");
        assert!(
            problems.iter().any(|p| p.contains("## Summary")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("must_have")),
            "{problems:?}"
        );
        assert!(
            problems.iter().any(|p| p.contains("run_budget_usd")),
            "{problems:?}"
        );
    }

    #[test]
    fn a_missing_section_is_named_and_never_guessed() {
        let form = b"# Spec Form: x\n\n## Summary\nbuild a thing\n\n## Scope\n- must_have:\n  - one thing\n\n## Budget\n- run_budget_usd: 2.5\n";
        let problems = check_form(form);
        assert!(
            problems.iter().any(|p| p.contains("## Success Criteria")),
            "{problems:?}"
        );
        assert!(!problems.iter().any(|p| p.contains("must_have")));
    }

    #[test]
    fn comments_are_not_answers() {
        // The Summary section that contains ONLY a comment is empty.
        let form = b"# Spec Form: x\n\n## Summary\n<!-- one sentence -->\n\n## Scope\n- must_have:\n  - one\n\n## Budget\n- run_budget_usd: 1\n";
        let problems = check_form(form);
        assert!(
            problems
                .iter()
                .any(|p| p.contains("'## Summary' has no content")),
            "{problems:?}"
        );
    }
}

/// parseable JSON object from surrounding prose, then re-check. Returns the
/// repaired Value or None when no object can be extracted.
///
/// Every `{` is a candidate start: prose often contains decoy braces
/// ("use the format {like this}") before the real payload, and the §14 repair
/// turn re-embeds the rejected text — bailing on the first candidate would
/// make repair structurally unable to reach the model's corrected object.
pub fn extract_json(text: &str) -> Option<Value> {
    let mut search = 0usize;
    while let Some(offset) = text[search..].find('{') {
        let start = search + offset;
        if let Some(value) = balanced_object(text, start) {
            return Some(value);
        }
        search = start + 1;
    }
    None
}

/// Find the object beginning at `start`, honouring strings and escapes, and
/// parse exactly that slice. `None` when unbalanced or not valid JSON.
fn balanced_object(text: &str, start: usize) -> Option<Value> {
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '\\' if in_string => {
                escaped = !escaped;
                continue;
            }
            '"' if !escaped => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return serde_json::from_str(&text[start..=start + i]).ok();
                }
            }
            _ => {}
        }
        escaped = false;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(p: &str) -> Value {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let txt = std::fs::read_to_string(root.join(p)).unwrap();
        serde_json::from_str(&txt).unwrap()
    }

    #[test]
    fn status_summary_contradicting_task_count_is_rejected() {
        // A plan claiming total=5 with 2 real tasks is a lying self-report.
        let mut doc = load("examples/expected/tasks_valid.json");
        doc["status_summary"]["total"] = serde_json::json!(5);
        let problems = validate_document("tasks.schema.json", &doc);
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("STATUS_SUMMARY_STALE")),
            "{problems:?}"
        );
        // The unmodified positive fixture (total=2, 2 tasks) passes.
        assert!(validate_document(
            "tasks.schema.json",
            &load("examples/expected/tasks_valid.json")
        )
        .is_empty());
    }

    #[test]
    fn traceability_passes_on_the_positive_fixtures() {
        let spec: Spec = serde_json::from_value(load("examples/expected/spec_valid.json")).unwrap();
        let plan: TaskPlan =
            serde_json::from_value(load("examples/expected/tasks_valid.json")).unwrap();
        assert_eq!(check_traceability(&spec, &plan), Vec::<String>::new());
    }

    #[test]
    fn traceability_rejects_uncovered_requirement() {
        // §6.4 cross-doc rule: spec declares R1+R2; drop every citation of R2
        // from the plan so it is genuinely uncovered. (NOTE: the fixture
        // `negative/tasks_uncovered_requirement.json` is schema-level — its
        // T2 has an EMPTY list, violating minItems:1, the *orphan-task* rule.
        // The cross-doc rule needs a spec+plan pair, exercised here.)
        let spec: Spec = serde_json::from_value(load("examples/expected/spec_valid.json")).unwrap();
        let mut plan: TaskPlan =
            serde_json::from_value(load("examples/expected/tasks_valid.json")).unwrap();
        for t in &mut plan.tasks {
            t.spec_requirement_ids.retain(|r| r != "R2");
        }
        // keep T1 schema-valid (still cites R1) so only R2 is uncovered
        let problems = check_traceability(&spec, &plan);
        assert_eq!(problems, vec!["TASK_UNCOVERED_REQUIREMENT: R2"]);
    }

    #[test]
    fn schema_full_rejects_orphan_task_with_no_requirement() {
        // The fixture's T2 has an empty spec_requirement_ids; the schema's
        // minItems:1 (enforced by validate_full/jsonschema, the structural
        // subset does not walk array items) is the guard against orphan tasks.
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let doc = load("examples/expected/negative/tasks_uncovered_requirement.json");
        let problems = validate_full(&root, "tasks.schema.json", &doc);
        assert!(
            problems.iter().any(|p| p.contains("spec_requirement_ids")),
            "orphan-task must fail full schema validation, got {problems:?}"
        );
    }

    #[test]
    fn traceability_rejects_dangling_requirement_reference() {
        let spec: Spec = serde_json::from_value(load("examples/expected/spec_valid.json")).unwrap();
        let mut plan: TaskPlan =
            serde_json::from_value(load("examples/expected/tasks_valid.json")).unwrap();
        plan.tasks[0].spec_requirement_ids.push("R999".into());
        let problems = check_traceability(&spec, &plan);
        assert!(
            problems
                .iter()
                .any(|p| p.starts_with("UNKNOWN_REQUIREMENT_REFERENCE")),
            "{problems:?}"
        );
    }

    #[test]
    fn traceability_flags_spec_id_mismatch() {
        let spec: Spec = serde_json::from_value(load("examples/expected/spec_valid.json")).unwrap();
        let mut plan: TaskPlan =
            serde_json::from_value(load("examples/expected/tasks_valid.json")).unwrap();
        plan.spec_id = "spec-forged".into();
        let problems = check_traceability(&spec, &plan);
        assert!(
            problems.iter().any(|p| p.starts_with("SPEC_ID_MISMATCH")),
            "{problems:?}"
        );
    }

    #[test]
    fn extract_json_pulls_object_from_prose() {
        let text = "Sure! Here is the object:
{\"id\": 1, \"note\": \"has } brace\"}
Hope it helps.";
        let v = extract_json(text).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["note"], "has } brace");
    }

    #[test]
    fn extract_json_rejects_unterminated() {
        assert!(extract_json("{\"a\": 1").is_none());
        assert!(extract_json("no json here").is_none());
    }

    #[test]
    fn extract_json_skips_decoy_braces_before_the_real_object() {
        // Regression: repair-turn prose routinely contains placeholder braces
        // ("use {like this}") before the payload; first-candidate bail-out
        // used to return None here.
        let text = "Please emit {\"field\": ...} — real object follows: {\"id\": 7, \"ok\": true}";
        let v = extract_json(text).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn extract_json_nested_objects_and_string_braces() {
        let text = r#"x { "a": {"b": "}"}, "c": [1,2] } y"#;
        let v = extract_json(text).unwrap();
        assert_eq!(v["a"]["b"], "}");
        assert_eq!(v["c"][1], 2);
    }

    #[test]
    fn extract_json_no_candidate_loops_forever_free() {
        // Every '{' must be consumed exactly once: pathological decoy text
        // terminates. (200 braces, no valid object.)
        let text = "{".repeat(200);
        assert!(extract_json(&text).is_none());
    }
}
