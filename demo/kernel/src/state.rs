//! Two state machines: one walks bytes, the other classifies a number.

/// Where the byte walker is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScanState {
    /// Outside quotes.
    #[default]
    Text,
    /// Inside a quoted run.
    Quoted,
    /// Just after a backslash inside a quoted run.
    Escape,
}

impl ScanState {
    /// The row of the transition table this state uses.
    pub const fn index(self) -> usize {
        match self {
            Self::Text => 0,
            Self::Quoted => 1,
            Self::Escape => 2,
        }
    }
}

/// What a byte walk found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanReport {
    /// Parenthesis depth at the end, counting `(` as one and `)` as minus one.
    pub depth: i64,
    /// How many brackets the walk saw outside quotes.
    pub punctuation: usize,
    /// Whether a quoted run was still open at the end.
    pub quoted: bool,
    /// Whether the input ended in a state that is not `Text` at depth zero.
    pub unbalanced: bool,
}

/// A transition of the byte walker.
type Transition = fn(ScanState, u8, &mut ScanReport) -> ScanState;

fn text_step(state: ScanState, byte: u8, report: &mut ScanReport) -> ScanState {
    match byte {
        b'(' | b'[' | b'{' => {
            report.depth += 1;
            report.punctuation += 1;
            state
        }
        b')' | b']' | b'}' => {
            report.depth -= 1;
            report.punctuation += 1;
            state
        }
        b'"' => ScanState::Quoted,
        _ => state,
    }
}

fn quoted_step(state: ScanState, byte: u8, report: &mut ScanReport) -> ScanState {
    match byte {
        b'"' => ScanState::Text,
        b'\\' => ScanState::Escape,
        _ => {
            report.punctuation += 1;
            state
        }
    }
}

fn escape_step(_state: ScanState, _byte: u8, _report: &mut ScanReport) -> ScanState {
    ScanState::Quoted
}

/// The table the walker dispatches through; the row is [`ScanState::index`].
const TRANSITIONS: [Transition; 3] = [text_step, quoted_step, escape_step];

/// Walks `text` and reports what it found.
pub fn scan(text: &str) -> ScanReport {
    let mut report = ScanReport::default();
    let mut state = ScanState::Text;
    for byte in text.bytes() {
        state = TRANSITIONS[state.index()](state, byte, &mut report);
    }
    report.quoted = state == ScanState::Quoted;
    report.unbalanced = report.depth != 0 || state != ScanState::Text;
    report
}

/// The band a number falls into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Below the warm threshold.
    Cold,
    /// At or above the warm threshold.
    Warm,
    /// At or above the hot threshold.
    Hot,
    /// At or above the critical threshold.
    Critical,
}

impl Level {
    /// The warm threshold.
    pub const WARM_AT: i64 = 10;
    /// The hot threshold.
    pub const HOT_AT: i64 = 100;
    /// The critical threshold.
    pub const CRITICAL_AT: i64 = 1000;

    /// The band `value` falls into.
    pub const fn of(value: i64) -> Self {
        // The guards are ordered, so the first one that holds is the answer.
        if value >= Self::CRITICAL_AT {
            Self::Critical
        } else if value >= Self::HOT_AT {
            Self::Hot
        } else if value >= Self::WARM_AT {
            Self::Warm
        } else {
            Self::Cold
        }
    }

    /// The name the band reports.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
            Self::Hot => "hot",
            Self::Critical => "critical",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_balanced_line_ends_in_text() {
        let report = scan("call(a, [b])");
        assert_eq!(report.depth, 0);
        assert_eq!(report.punctuation, 4);
        assert!(!report.unbalanced);
        assert!(!report.quoted);
    }

    #[test]
    fn brackets_inside_quotes_do_not_count() {
        let report = scan("\"(\"");
        assert_eq!(report.depth, 0);
        assert_eq!(report.punctuation, 1);
        assert!(!report.unbalanced);
    }

    #[test]
    fn an_open_quote_or_bracket_is_reported() {
        assert!(scan("\"open").unbalanced);
        assert!(scan("(open").unbalanced);
        assert!(!scan("(open)").unbalanced);
        // A backslash at the end leaves the walker in Escape.
        assert!(scan("\"a\\").unbalanced);
    }

    #[test]
    fn an_escaped_quote_does_not_close_the_run() {
        let report = scan("\"a\\\"b\"");
        assert!(!report.unbalanced);
        // The backslash and the quote after it are one escape; `a` and `b` are
        // the two characters inside the run.
        assert_eq!(report.punctuation, 2);
    }

    #[test]
    fn bands_are_ordered() {
        assert_eq!(Level::of(-5), Level::Cold);
        assert_eq!(Level::of(10), Level::Warm);
        assert_eq!(Level::of(100), Level::Hot);
        assert_eq!(Level::of(i64::MAX), Level::Critical);
        assert_eq!(Level::Hot.name(), "hot");
    }
}
