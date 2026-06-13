//! `pulse indicators` offline indicator-series viewer.

use std::path::PathBuf;

use crate::{
    CandleStore, CompiledValue, EvalContext, IndicatorEngine, IndicatorSpec, Pair, SweepableValue,
};
use rust_decimal::Decimal;

use super::parse_one_tf;

const BLANK: &str = "—";

/// `pulse indicators --pair BTCUSDT --tf M15 [--indicator rsi:14] [--limit N]`.
#[derive(Debug, clap::Args)]
pub struct IndicatorsArgs {
    /// The trading pair symbol (e.g. `BTCUSDT`).
    #[arg(long)]
    pub pair: String,
    /// Candle timeframe to load (`M15`/`15m` or `H4`/`4h`).
    #[arg(long)]
    pub tf: String,
    /// `CandleStore` root. Defaults to the committed `BTCUSDT` fixture under CWD.
    #[arg(long)]
    pub base_dir: Option<PathBuf>,
    /// Indicator to render as `<kind>:<period>`; repeatable.
    #[arg(long = "indicator")]
    pub indicators: Vec<String>,
    /// Maximum number of candle rows to print. Omitted means all rows.
    #[arg(long)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IndicatorColumn {
    pub(crate) label: String,
    pub(crate) spec: IndicatorSpec,
}

/// Load a local snapshot, stream the indicator engine over every candle, and
/// print a deterministic tab-separated series.
///
/// # Errors
///
/// Returns an [`anyhow::Error`] on invalid args, missing fixture data, or engine
/// construction failure.
pub fn run_indicators(args: &IndicatorsArgs) -> anyhow::Result<()> {
    let pair = Pair::parse(args.pair.clone())
        .map_err(|e| anyhow::anyhow!("invalid pair argument: {e}"))?;
    let tf = parse_one_tf(&args.tf)?;
    let base_dir = match &args.base_dir {
        Some(path) => path.clone(),
        None => default_fixture_base_dir()?,
    };
    let store = CandleStore::with_base_dir(base_dir);
    let head = store
        .read_head(&pair, tf)?
        .ok_or_else(|| anyhow::anyhow!("no HEAD snapshot for {pair} {}", tf.binance_interval()))?;
    let series = store.read_snapshot(&pair, tf, &head)?;
    let columns = parse_indicator_specs(&args.indicators)?;
    let specs = columns
        .iter()
        .map(|column| column.spec.clone())
        .collect::<Vec<_>>();
    let mut engine = IndicatorEngine::from_specs(&specs)?;
    let mut first_rows = vec![None; columns.len()];

    println!("{}", render_header(&columns));
    for (idx, candle) in series.candles.iter().enumerate() {
        engine.step(candle);
        let values = current_values(&engine, &columns);
        note_first_rows(idx + 1, &values, &mut first_rows);
        if args.limit.is_none_or(|limit| idx < limit) {
            println!("{}", render_row(candle.open_time, &values));
        }
    }
    println!(
        "{}",
        render_summary(series.candles.len(), &columns, &first_rows)
    );
    Ok(())
}

pub(crate) fn parse_indicator_specs(raw: &[String]) -> anyhow::Result<Vec<IndicatorColumn>> {
    let tokens = if raw.is_empty() {
        ["rsi:14", "ema:50", "adx:14"]
            .iter()
            .map(|token| (*token).to_owned())
            .collect::<Vec<_>>()
    } else {
        raw.to_vec()
    };

    // The engine dedups specs, but the viewer renders one column per flag — so
    // repeated `--indicator` flags would print duplicate columns. Dedup here,
    // order-preserving, keyed on the case-normalized `kind:period` label (which
    // is 1:1 with the parsed spec for rsi/ema/adx).
    let mut seen = std::collections::HashSet::new();
    let mut columns = Vec::with_capacity(tokens.len());
    for token in &tokens {
        let column = parse_one_indicator(token)?;
        if seen.insert(column.label.clone()) {
            columns.push(column);
        }
    }
    Ok(columns)
}

#[must_use]
pub(crate) fn render_indicator_value(value: Option<Decimal>) -> String {
    value.map_or_else(|| BLANK.to_owned(), |value| value.normalize().to_string())
}

#[must_use]
pub(crate) fn render_header(columns: &[IndicatorColumn]) -> String {
    let mut cells = Vec::with_capacity(columns.len() + 1);
    cells.push("open_time".to_owned());
    cells.extend(columns.iter().map(|column| column.label.clone()));
    cells.join("\t")
}

#[must_use]
pub(crate) fn render_row(open_time: i64, values: &[Option<Decimal>]) -> String {
    let mut cells = Vec::with_capacity(values.len() + 1);
    cells.push(open_time.to_string());
    cells.extend(values.iter().copied().map(render_indicator_value));
    cells.join("\t")
}

fn default_fixture_base_dir() -> anyhow::Result<PathBuf> {
    Ok(std::env::current_dir()
        .map_err(|e| anyhow::anyhow!("resolve current directory for default fixture: {e}"))?
        .join("tests/fixtures/btcusdt-1m-store"))
}

fn parse_one_indicator(token: &str) -> anyhow::Result<IndicatorColumn> {
    let (kind, period) = token.split_once(':').ok_or_else(|| {
        anyhow::anyhow!("invalid --indicator {token:?}: expected <kind>:<period>")
    })?;
    let kind = kind.trim().to_ascii_lowercase();
    let period = parse_period(token, period)?;
    let fixed = SweepableValue::Fixed(period);
    let spec = match kind.as_str() {
        "rsi" => IndicatorSpec::Rsi { period: fixed },
        "ema" => IndicatorSpec::Ema { period: fixed },
        "adx" => IndicatorSpec::Adx { period: fixed },
        "macd" => anyhow::bail!(
            "invalid --indicator {token:?}: MACD needs fast/slow/signal and is not supported by <kind>:<period>"
        ),
        _ => anyhow::bail!(
            "invalid --indicator {token:?}: unknown kind {kind:?} (expected rsi, ema, or adx)"
        ),
    };
    Ok(IndicatorColumn {
        label: format!("{kind}:{period}"),
        spec,
    })
}

fn parse_period(token: &str, period: &str) -> anyhow::Result<u32> {
    let period = period
        .trim()
        .parse::<u32>()
        .map_err(|e| anyhow::anyhow!("invalid --indicator {token:?}: period must be u32: {e}"))?;
    if period == 0 {
        anyhow::bail!("invalid --indicator {token:?}: period must be >= 1");
    }
    Ok(period)
}

fn current_values(engine: &IndicatorEngine, columns: &[IndicatorColumn]) -> Vec<Option<Decimal>> {
    columns
        .iter()
        .map(|column| engine.current(&CompiledValue::Indicator(column.spec.clone())))
        .collect()
}

fn note_first_rows(row: usize, values: &[Option<Decimal>], first_rows: &mut [Option<usize>]) {
    for (first_row, value) in first_rows.iter_mut().zip(values) {
        if first_row.is_none() && value.is_some() {
            *first_row = Some(row);
        }
    }
}

fn render_summary(
    candle_count: usize,
    columns: &[IndicatorColumn],
    first_rows: &[Option<usize>],
) -> String {
    let mut cells = Vec::with_capacity(columns.len() + 2);
    cells.push("summary".to_owned());
    cells.push(format!("candles={candle_count}"));
    cells.extend(columns.iter().zip(first_rows).map(|(column, first_row)| {
        format!(
            "{}_first_row={}",
            column.label,
            first_row.map_or_else(|| "none".to_owned(), |row| row.to_string())
        )
    }));
    cells.join("\t")
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::{parse_indicator_specs, render_header, render_indicator_value, render_row};
    use crate::{IndicatorSpec, SweepableValue};
    use rust_decimal::Decimal;

    fn fixed_period(spec: &IndicatorSpec) -> u32 {
        match spec {
            IndicatorSpec::Rsi { period }
            | IndicatorSpec::Ema { period }
            | IndicatorSpec::Adx { period } => match period {
                SweepableValue::Fixed(period) => *period,
                SweepableValue::Sweep { .. } => panic!("CLI specs must be fixed"),
            },
            IndicatorSpec::Macd { .. } => panic!("MACD is not part of kind:period parsing"),
        }
    }

    #[test]
    fn parses_indicator_flag_kind_and_period() {
        let one = parse_indicator_specs(&["rsi:14".to_owned()]).expect("rsi:14 parses");
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].label, "rsi:14");
        assert!(matches!(one[0].spec, IndicatorSpec::Rsi { .. }));
        assert_eq!(fixed_period(&one[0].spec), 14);

        let repeated =
            parse_indicator_specs(&["ema:50".to_owned(), "adx:14".to_owned()]).expect("parse");
        assert_eq!(repeated.len(), 2);
        assert_eq!(repeated[0].label, "ema:50");
        assert!(matches!(repeated[0].spec, IndicatorSpec::Ema { .. }));
        assert_eq!(fixed_period(&repeated[0].spec), 50);
        assert_eq!(repeated[1].label, "adx:14");
        assert!(matches!(repeated[1].spec, IndicatorSpec::Adx { .. }));
        assert_eq!(fixed_period(&repeated[1].spec), 14);

        let defaults = parse_indicator_specs(&[]).expect("defaults parse");
        assert_eq!(
            defaults
                .iter()
                .map(|column| column.label.as_str())
                .collect::<Vec<_>>(),
            ["rsi:14", "ema:50", "adx:14"]
        );

        // Repeated / case-variant specs dedup to one column each, order preserved
        // (the viewer renders one column per surviving spec).
        let deduped = parse_indicator_specs(&[
            "rsi:14".to_owned(),
            "RSI:14".to_owned(),
            "ema:50".to_owned(),
            "rsi:14".to_owned(),
        ])
        .expect("dedup parses");
        assert_eq!(
            deduped
                .iter()
                .map(|column| column.label.as_str())
                .collect::<Vec<_>>(),
            ["rsi:14", "ema:50"],
            "duplicate / case-variant --indicator flags dedup, order preserved"
        );

        assert!(parse_indicator_specs(&["macd:12".to_owned()]).is_err());
        assert!(parse_indicator_specs(&["bogus:14".to_owned()]).is_err());
        assert!(parse_indicator_specs(&["rsi".to_owned()]).is_err());
        assert!(parse_indicator_specs(&["rsi:0".to_owned()]).is_err());
    }

    #[test]
    fn warmup_rows_render_as_blank() {
        let columns =
            parse_indicator_specs(&["rsi:14".to_owned(), "ema:50".to_owned()]).expect("parse");
        assert_eq!(render_header(&columns), "open_time\trsi:14\tema:50");
        assert_eq!(render_indicator_value(None), "—");
        assert_eq!(
            render_indicator_value(Some(Decimal::new(42_125, 3))),
            "42.125"
        );
        assert_eq!(
            render_row(1_700_000_000_000, &[None, Some(Decimal::new(42_125, 3))]),
            "1700000000000\t—\t42.125"
        );
    }
}
