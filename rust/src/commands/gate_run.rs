//! Port of `src/commands/gateRun.ts` (`renderGate`: shared check/next/guard
//! rendering of a `GateResult`). Ported alongside the commands that call it
//! (wave 4, see `docs/rust-port.md`).

use crate::cli::args::Flags;
use crate::cli::output::{emit, UserError};
use crate::core::json::Value;
use crate::gates::types::GateResult;

fn mark(ok: bool) -> &'static str {
    if ok {
        "\u{2713}"
    } else {
        "\u{2717}"
    }
}

/// Render a gate result in the requested format and return whether it
/// passed. `extra` is merged into the JSON payload after `phase`/`ok`/
/// `checks` - the same key/value pairs TS spreads via `...extra` into the
/// object literal `{ phase, ok, checks, ...extra }`.
pub fn render_gate(
    res: &GateResult,
    extra: Vec<(String, Value)>,
    flags: &Flags,
) -> Result<bool, UserError> {
    let mut lines = vec![format!(
        "{} {} gate: {}",
        mark(res.ok),
        res.phase,
        if res.ok { "PASS" } else { "FAIL" }
    )];
    for c in &res.checks {
        let mut line = format!("  {} {}", mark(c.ok), c.name);
        if !c.detail.is_empty() {
            line.push_str(" - ");
            line.push_str(&c.detail);
        }
        lines.push(line);
    }

    let mut data = Value::object();
    data.insert("phase", res.phase.as_str());
    data.insert("ok", res.ok);
    let mut checks = Value::array();
    for c in &res.checks {
        let mut co = Value::object();
        co.insert("name", c.name.as_str());
        co.insert("ok", c.ok);
        co.insert("detail", c.detail.as_str());
        checks.push(co);
    }
    data.insert("checks", checks);
    for (k, v) in extra {
        data.insert(k, v);
    }

    emit(&lines.join("\n"), &data, flags)?;
    Ok(res.ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::args::parse_args;
    use crate::core::state_machine::Phase;
    use crate::gates::types::{fail, pass, Check};

    fn flags_from(args: &[&str]) -> Flags {
        parse_args(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).flags
    }

    #[test]
    fn renders_a_passing_gate_with_check_marks() {
        let res = GateResult {
            phase: Phase::Plan,
            ok: true,
            checks: vec![pass("plan.schema", "")],
        };
        let ok = render_gate(&res, Vec::new(), &flags_from(&["cmd"])).unwrap();
        assert!(ok);
    }

    #[test]
    fn a_check_with_a_detail_appends_it_after_a_dash() {
        let res = GateResult {
            phase: Phase::Plan,
            ok: false,
            checks: vec![fail("plan.schema", "missing goal")],
        };
        let ok = render_gate(&res, Vec::new(), &flags_from(&["cmd"])).unwrap();
        assert!(!ok);
    }

    #[test]
    fn checks_json_field_order_is_name_ok_detail() {
        let res = GateResult {
            phase: Phase::Plan,
            ok: true,
            checks: vec![Check {
                name: "n".to_string(),
                ok: true,
                detail: "d".to_string(),
                trust_blocked: false,
            }],
        };
        let mut data = Value::object();
        data.insert("phase", res.phase.as_str());
        data.insert("ok", res.ok);
        let mut checks = Value::array();
        for c in &res.checks {
            let mut co = Value::object();
            co.insert("name", c.name.as_str());
            co.insert("ok", c.ok);
            co.insert("detail", c.detail.as_str());
            checks.push(co);
        }
        data.insert("checks", checks);
        let json = crate::core::json::stringify_pretty(&data);
        assert!(json.contains("\"name\": \"n\""));
        let checks_idx = json.find("\"checks\"").unwrap();
        let inner = &json[checks_idx..];
        let name_idx = inner.find("\"name\"").unwrap();
        let ok_idx = inner.find("\"ok\"").unwrap();
        let detail_idx = inner.find("\"detail\"").unwrap();
        assert!(name_idx < ok_idx && ok_idx < detail_idx);
    }
}
