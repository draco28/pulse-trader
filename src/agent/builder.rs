//! `StrategyBuilder` — the partial-strategy accumulator the six builder tools
//! mutate (VS-1.3.2 work-2.02, FR-3 heart).
//!
//! A stateful, order-independent accumulator of the DSL pieces a composer loop
//! (2.04) stages across a tool-call sequence. [`StrategyBuilder::finalize`]
//! assembles the accumulated fields into a [`StrategyDsl`] (server-owned
//! `schema_version`) and runs the **existing** whole-document
//! [`validate`](crate::domain::validate) (VS-1.1.2) — no validation rule is
//! reimplemented here. Pure domain-adjacent logic: no LLM, no I/O, no streaming.
//!
//! The mutation surface is a set of `pub(crate)` setters the sibling
//! [`tools`](super::tools) module calls after it has mapped a flat-primitive
//! tool argument into a tagged DSL fragment: `create_strategy`/`add_entry_signal`
//! /`set_exit_rules`/`set_risk_params` **replace/set** their field, `add_filter`
//! **appends**, and only [`finalize`](StrategyBuilder::finalize) checks
//! completeness (order-independence + idempotency, audit F3).

// The builder is built-but-unwired this slice: its first production caller is the
// composer loop (2.04, R3), so `new()` (and the re-exported surface) is otherwise
// `dead_code` under `deny(warnings)` — the VS-1.3.1 harvested dead-code gotcha.
#![allow(dead_code)]

use crate::domain::{
    Condition, Direction, ExitRule, FieldError, RiskParams, SchemaVersion, StrategyDsl,
    ValidatedDsl, ValidationCode, ValidationErrors, validate,
};

/// A partial strategy, accumulated across the tool-call loop and assembled +
/// fully validated at [`finalize`](StrategyBuilder::finalize).
///
/// Reuses the existing DSL vocabulary ([`Condition`]/[`ExitRule`]/[`RiskParams`]
/// /[`Direction`]) — this item adds **no** new DSL grammar. No `#[derive(Debug)]`
/// (habit from the VS-1.3.1 secrets gotcha; nothing secret here).
pub(crate) struct StrategyBuilder {
    name: Option<String>,
    direction: Option<Direction>,
    entry: Option<Condition>,
    filters: Vec<Condition>,
    exits: Vec<ExitRule>,
    risk: Option<RiskParams>,
}

impl StrategyBuilder {
    /// An empty builder — no field set, no filters or exits accumulated.
    pub(crate) fn new() -> Self {
        Self {
            name: None,
            direction: None,
            entry: None,
            filters: Vec::new(),
            exits: Vec::new(),
            risk: None,
        }
    }

    /// Set (replacing) the strategy name + direction (`create_strategy`).
    pub(crate) fn set_identity(&mut self, name: String, direction: Direction) {
        self.name = Some(name);
        self.direction = Some(direction);
    }

    /// Set (replacing) the entry trigger (`add_entry_signal`).
    pub(crate) fn set_entry(&mut self, entry: Condition) {
        self.entry = Some(entry);
    }

    /// Append an AND-conjoined filter (`add_filter`).
    pub(crate) fn push_filter(&mut self, filter: Condition) {
        self.filters.push(filter);
    }

    /// Set (replacing) the exit rules (`set_exit_rules`).
    pub(crate) fn set_exits(&mut self, exits: Vec<ExitRule>) {
        self.exits = exits;
    }

    /// Set (replacing) the risk / sizing params (`set_risk_params`).
    pub(crate) fn set_risk(&mut self, risk: RiskParams) {
        self.risk = Some(risk);
    }

    /// Assemble the accumulated fields into a [`StrategyDsl`] (server-owned
    /// `schema_version = SchemaVersion::CURRENT`) and run the whole-document
    /// [`validate`](crate::domain::validate).
    ///
    /// This is the FR-4 payoff 2.04 reads to build a `StrategyVersion`.
    ///
    /// # Errors
    ///
    /// Returns a non-empty `Vec<FieldError>` when a required piece is unset
    /// (`name`/`direction`/`entry`/`exits`/`risk`) — a correctable "call X first"
    /// message — or when the assembled document fails semantic validation (the
    /// whole-document rules, mapped from [`validate`](crate::domain::validate)'s
    /// error collection).
    pub(crate) fn finalize(&self) -> Result<ValidatedDsl, Vec<FieldError>> {
        let mut errors = Vec::new();
        if self.name.is_none() {
            errors.push(missing(
                "name",
                "call create_strategy to set the strategy name",
            ));
        }
        if self.direction.is_none() {
            errors.push(missing(
                "direction",
                "call create_strategy to set the trade direction",
            ));
        }
        if self.entry.is_none() {
            errors.push(missing(
                "entry",
                "call add_entry_signal to set the entry trigger",
            ));
        }
        if self.exits.is_empty() {
            errors.push(FieldError {
                path: "exits".to_owned(),
                code: ValidationCode::NoExit,
                message: "call set_exit_rules to set at least one exit rule".to_owned(),
            });
        }
        if self.risk.is_none() {
            errors.push(missing("risk", "call set_risk_params to set risk sizing"));
        }
        let (Some(name), Some(direction), Some(entry), Some(risk)) = (
            self.name.clone(),
            self.direction,
            self.entry.clone(),
            self.risk.clone(),
        ) else {
            return Err(errors);
        };
        if !errors.is_empty() {
            // A required piece other than name/direction/entry/risk is missing
            // (an empty exits list); do not assemble a partial document.
            return Err(errors);
        }
        let dsl = StrategyDsl {
            schema_version: SchemaVersion::CURRENT,
            name,
            direction,
            entry,
            filters: self.filters.clone(),
            exits: self.exits.clone(),
            risk,
        };
        // Reuse the existing whole-document validator verbatim (VS-1.1.2); its
        // guaranteed-non-empty error collection maps to the correctable Vec.
        validate(&dsl).map_err(ValidationErrors::into_errors)
    }
}

/// A correctable "required piece missing" error (path = the missing field,
/// `FieldRange` code per the work-2.02 decision, a "call X first" message).
fn missing(path: &str, message: &str) -> FieldError {
    FieldError {
        path: path.to_owned(),
        code: ValidationCode::FieldRange,
        message: message.to_owned(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::StrategyBuilder;
    use crate::domain::{
        Comparator, Condition, Direction, ExitRule, IndicatorSpec, PriceField, RiskParams,
        SchemaVersion, SweepableValue, ValidationCode, ValueSource,
    };
    use rust_decimal::Decimal;

    fn valid_entry() -> Condition {
        Condition::Compare {
            lhs: ValueSource::Indicator {
                spec: IndicatorSpec::Rsi {
                    period: SweepableValue::Fixed(14),
                },
            },
            op: Comparator::Lt,
            rhs: ValueSource::Constant {
                value: Decimal::new(30, 0),
            },
        }
    }

    fn ema_filter(period: u32) -> Condition {
        Condition::Compare {
            lhs: ValueSource::Price {
                field: PriceField::Close,
            },
            op: Comparator::Gt,
            rhs: ValueSource::Indicator {
                spec: IndicatorSpec::Ema {
                    period: SweepableValue::Fixed(period),
                },
            },
        }
    }

    fn valid_exits() -> Vec<ExitRule> {
        vec![
            ExitRule::StopLoss {
                distance_pct: SweepableValue::Fixed(Decimal::new(5, 2)),
            },
            ExitRule::TakeProfit {
                target_r: SweepableValue::Fixed(Decimal::new(2, 0)),
            },
        ]
    }

    fn valid_risk() -> RiskParams {
        RiskParams {
            risk_per_trade_pct: SweepableValue::Fixed(Decimal::new(1, 2)),
            max_leverage: SweepableValue::Fixed(Decimal::new(3, 0)),
        }
    }

    #[test]
    fn new_builder_finalize_reports_all_missing_pieces() {
        let builder = StrategyBuilder::new();
        let errs = builder
            .finalize()
            .expect_err("an empty builder cannot finalize");
        assert!(errs.iter().any(|e| e.path == "name"));
        assert!(errs.iter().any(|e| e.path == "direction"));
        assert!(errs.iter().any(|e| e.path == "entry"));
        assert!(
            errs.iter()
                .any(|e| e.path == "exits" && e.code == ValidationCode::NoExit)
        );
        assert!(errs.iter().any(|e| e.path == "risk"));
    }

    #[test]
    fn builder_finalize_assembles_and_validates() {
        let mut builder = StrategyBuilder::new();
        builder.set_identity("RSI Oversold".to_owned(), Direction::Long);
        builder.set_entry(valid_entry());
        builder.set_exits(valid_exits());
        builder.set_risk(valid_risk());
        let validated = builder
            .finalize()
            .expect("a complete valid builder finalizes");
        assert_eq!(validated.dsl().name, "RSI Oversold");
        assert_eq!(validated.dsl().direction, Direction::Long);
        assert_eq!(validated.dsl().schema_version, SchemaVersion::CURRENT);
    }

    #[test]
    fn builder_push_filter_appends_in_order() {
        let mut builder = StrategyBuilder::new();
        builder.set_identity("filtered".to_owned(), Direction::Long);
        builder.set_entry(valid_entry());
        builder.push_filter(ema_filter(50));
        builder.push_filter(ema_filter(200));
        builder.set_exits(valid_exits());
        builder.set_risk(valid_risk());
        let validated = builder.finalize().expect("finalizes with two filters");
        assert_eq!(validated.dsl().filters.len(), 2);
    }

    #[test]
    fn builder_finalize_missing_entry_is_correctable() {
        let mut builder = StrategyBuilder::new();
        builder.set_identity("no entry".to_owned(), Direction::Short);
        builder.set_exits(valid_exits());
        builder.set_risk(valid_risk());
        let errs = builder
            .finalize()
            .expect_err("a missing entry must not validate");
        assert!(errs.iter().any(|e| e.path == "entry"));
    }
}
