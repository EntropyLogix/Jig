use crate::clock;
use crate::issue;
use crate::memory;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextKind {
    Id,
    Label,
    Release,
    Topic,
    Date,
    Note,
    Multiline,
    Body,
    Free,
}

impl TextKind {
    pub const ALL: [TextKind; 9] = [
        TextKind::Id,
        TextKind::Label,
        TextKind::Release,
        TextKind::Topic,
        TextKind::Date,
        TextKind::Note,
        TextKind::Multiline,
        TextKind::Body,
        TextKind::Free,
    ];

    pub fn check(self, field: &str, required: bool, value: &str) -> Result<(), String> {
        let line_break = |c: char| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}');
        match self {
            TextKind::Id if issue::id_alias(value).is_none() => {
                Err(format!("invalid issue id {value:?}"))
            }
            TextKind::Label if !issue::valid_label(value) => {
                Err(format!("{field} must be a non-empty single line"))
            }
            TextKind::Release if !issue::valid_release(value) => Err(
                "release must be a non-empty single line without surrounding whitespace".into(),
            ),
            TextKind::Topic if !memory::valid_topic(value) => {
                Err(format!("invalid topic {value:?}"))
            }
            TextKind::Date if !clock::valid_date(value) => {
                Err(format!("invalid defer date {value:?}"))
            }
            TextKind::Note if value.is_empty() => Err("note text is empty".into()),
            TextKind::Note if value.chars().any(line_break) => Err(
                "note text must be a single line without control characters or Unicode line separators"
                    .into(),
            ),
            TextKind::Note if value.chars().count() > 500 => Err(format!(
                "note has {} characters; maximum is 500",
                value.chars().count()
            )),
            TextKind::Multiline if required && value.trim().is_empty() => {
                Err(format!("{field} is required"))
            }
            TextKind::Multiline if value.chars().any(|c| c != '\n' && c.is_control()) => Err(
                format!("{field}: control characters other than newline are not allowed"),
            ),
            TextKind::Multiline => issue::structural_check(value, field),
            TextKind::Body => issue::structural_check(value, "body"),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
pub(crate) mod generated {
    pub(crate) const STRUCT: u16 = 1;
    pub(crate) const NL: u16 = 2;
    pub(crate) const CTRL: u16 = 4;
    pub(crate) const SEP: u16 = 8;
    pub(crate) const BLANK: u16 = 16;
    pub(crate) const EMPTY: u16 = 32;
    pub(crate) const LONG: u16 = 64;
    pub(crate) const PAD: u16 = 128;

    pub(crate) struct Rng(pub(crate) u64);

    impl Rng {
        pub(crate) fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0 >> 33
        }

        pub(crate) fn below(&mut self, n: usize) -> usize {
            self.next() as usize % n
        }
    }

    const FRAGMENTS: &[(&str, u16)] = &[
        ("plain", 0),
        ("zażółć 🎲 ünïcode — dash", 0),
        ("\"quoted\" 'apos' `tick`", 0),
        ("--force", 0),
        ("-", 0),
        ("---", 0),
        ("# heading", 0),
        ("\n", NL),
        ("\r\n", NL | CTRL),
        ("\t", CTRL),
        ("\u{1b}", CTRL),
        ("\u{85}", CTRL),
        ("\u{0}", CTRL),
        ("\u{7f}", CTRL),
        ("\u{2028}", SEP),
        ("\u{2029}", SEP),
        ("\n## History\n", NL | STRUCT),
        ("\n   ## Notes ###\n", NL | STRUCT),
        ("\n## Closing Summary\n", NL | STRUCT),
        (
            "\n<<<<<<< ours\nx\n=======\ny\n>>>>>>> theirs\n",
            NL | STRUCT,
        ),
        (
            "\n```\n## History\n<<<<<<< a\n=======\n>>>>>>> b\n```\n",
            NL,
        ),
        ("\n~~~~\n## Notes\n~~~~\n", NL),
        ("\n> ## Notes\n", NL),
        ("\n    ## History\n", NL),
    ];

    pub(crate) fn text(rng: &mut Rng) -> (String, u16) {
        let mut out = String::new();
        let mut tags = 0;
        match rng.below(8) {
            0 => return (String::new(), BLANK | EMPTY),
            1 => {
                let spaces = " ".repeat(1 + rng.below(3));
                return (spaces, BLANK);
            }
            _ => {}
        }
        let pad = rng.below(6) == 0;
        if pad {
            out.push(' ');
        }
        for _ in 0..1 + rng.below(4) {
            let (fragment, tag) = FRAGMENTS[rng.below(FRAGMENTS.len())];
            out.push_str(fragment);
            tags |= tag;
        }
        if rng.below(10) == 0 {
            out.push_str(&"x".repeat(500));
        }
        if rng.below(10) == 0 {
            out.push_str("\n```\nunclosed");
            tags |= NL | STRUCT;
        }
        if pad {
            out.push(' ');
        }
        if out.trim().is_empty() {
            tags |= BLANK;
        }
        if out.trim() != out {
            tags |= PAD;
        }
        if out.chars().count() > 500 {
            tags |= LONG;
        }
        (out, tags)
    }
}

#[cfg(test)]
mod tests {
    use super::TextKind;
    use super::generated::{self, BLANK, CTRL, EMPTY, LONG, NL, PAD, Rng, SEP, STRUCT};

    fn accepts(kind: TextKind, required: bool, tags: u16) -> bool {
        let refused = match kind {
            TextKind::Label => NL | CTRL | SEP | BLANK,
            TextKind::Release => NL | CTRL | SEP | BLANK | PAD,
            TextKind::Note => NL | CTRL | SEP | EMPTY | LONG,
            TextKind::Multiline if required => STRUCT | CTRL | BLANK,
            TextKind::Multiline => STRUCT | CTRL,
            TextKind::Body => STRUCT,
            TextKind::Free => 0,
            TextKind::Id | TextKind::Topic | TextKind::Date => unreachable!(),
        };
        tags & refused == 0
    }

    #[test]
    fn free_text_kinds_follow_their_class_rules() {
        let kinds = [
            (TextKind::Label, false),
            (TextKind::Release, false),
            (TextKind::Note, true),
            (TextKind::Multiline, true),
            (TextKind::Multiline, false),
            (TextKind::Body, false),
            (TextKind::Free, true),
        ];
        let mut rng = Rng(0x5eed_7e47);
        let mut seen = [[0usize; 2]; 7];
        for case in 0..4000 {
            let (value, tags) = generated::text(&mut rng);
            for (index, (kind, required)) in kinds.iter().enumerate() {
                let expected = accepts(*kind, *required, tags);
                let actual = kind.check("--field", *required, &value);
                assert_eq!(
                    actual.is_ok(),
                    expected,
                    "seed 0x5eed7e47 case {case}: {kind:?} required={required} tags={tags:#b} value={value:?} -> {actual:?}"
                );
                seen[index][usize::from(expected)] += 1;
            }
        }
        for (index, (kind, required)) in kinds.iter().enumerate() {
            let refusals_possible = *kind != TextKind::Free;
            assert!(
                seen[index][1] > 0 && (seen[index][0] > 0 || !refusals_possible),
                "{kind:?} required={required} saw {:?}",
                seen[index]
            );
        }
    }

    #[test]
    fn refusals_name_the_field_or_the_rule() {
        let cases = [
            (TextKind::Label, "", "--by must be a non-empty single line"),
            (
                TextKind::Release,
                " 1.0",
                "release must be a non-empty single line without surrounding whitespace",
            ),
            (TextKind::Note, "", "note text is empty"),
            (
                TextKind::Note,
                "a\nb",
                "note text must be a single line without control characters or Unicode line separators",
            ),
            (TextKind::Multiline, " ", "--by is required"),
            (
                TextKind::Multiline,
                "a\tb",
                "--by: control characters other than newline are not allowed",
            ),
            (
                TextKind::Multiline,
                "## Notes",
                "--by: reserved heading ## Notes",
            ),
            (TextKind::Body, "```\nx", "body: unclosed code fence"),
            (TextKind::Id, "B-ZZZZZZ", "invalid issue id \"B-ZZZZZZ\""),
            (TextKind::Topic, "-x", "invalid topic \"-x\""),
            (
                TextKind::Date,
                "2026-02-29",
                "invalid defer date \"2026-02-29\"",
            ),
        ];
        for (kind, value, message) in cases {
            assert_eq!(kind.check("--by", true, value), Err(message.to_string()));
        }
        assert_eq!(
            TextKind::Note.check("text", true, &"y".repeat(501)),
            Err("note has 501 characters; maximum is 500".to_string())
        );
        assert_eq!(TextKind::Note.check("text", true, &"y".repeat(500)), Ok(()));
    }

    fn days(year: u32, month: u32) -> u32 {
        let leap =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ][month as usize - 1]
    }

    #[test]
    fn format_kinds_accept_exactly_their_grammar() {
        let mut rng = Rng(0xf0_4a75);
        let hex = b"0123456789abcdef";
        for case in 0..3000 {
            let stem: String = (0..6).map(|_| hex[rng.below(16)] as char).collect();
            let suffix: String = (0..rng.below(3))
                .map(|_| format!(".{}", 1 + rng.below(30)))
                .collect();
            let id = format!("B-{stem}{suffix}");
            let legacy = format!("B{}{suffix}", 1 + rng.below(9999));
            for valid in [id.clone(), legacy, format!("{stem}{suffix}")] {
                assert_eq!(
                    TextKind::Id.check("id", true, &valid),
                    Ok(()),
                    "case {case} {valid:?}"
                );
            }
            let mut broken = vec![
                format!("B-{}", &stem[..5]),
                format!("B-{stem}0"),
                format!("B-{stem}.0"),
                format!("B-{stem}.07"),
                format!("B-{stem}."),
                format!("b-{stem}"),
                format!("-{id}"),
                format!("{id} "),
            ];
            if stem.bytes().any(|c| c.is_ascii_alphabetic()) {
                broken.push(format!("B-{}", stem.to_uppercase()));
            }
            for invalid in broken {
                assert!(
                    TextKind::Id.check("id", true, &invalid).is_err(),
                    "case {case} {invalid:?}"
                );
            }

            let year = 1 + rng.below(9999) as u32;
            let month = 1 + rng.below(12) as u32;
            let last = days(year, month);
            let day = 1 + rng.below(31) as u32;
            let date = format!("{year:04}-{month:02}-{day:02}");
            assert_eq!(
                TextKind::Date.check("--until", false, &date).is_ok(),
                day <= last,
                "case {case} {date}"
            );
            for invalid in [
                format!("{year:04}-{:02}-01", month + 12),
                format!("{year:04}-{month:02}-00"),
                format!("{year:04}-{month}-1"),
                format!("{year:04}/{month:02}/01"),
                format!("{date}T00:00:00Z"),
            ] {
                assert!(
                    TextKind::Date.check("--until", false, &invalid).is_err(),
                    "case {case} {invalid}"
                );
            }

            let alphabet = b"abcXYZ019._-";
            let first = b"aZ0"[rng.below(3)] as char;
            let rest: String = (0..rng.below(12))
                .map(|_| alphabet[rng.below(alphabet.len())] as char)
                .collect();
            let topic = format!("{first}{rest}");
            assert_eq!(
                TextKind::Topic.check("topic", true, &topic),
                Ok(()),
                "case {case} {topic}"
            );
            for invalid in [
                format!(".{topic}"),
                format!("-{topic}"),
                format!("{topic} x"),
                format!("{topic}/x"),
                format!("{topic}ł"),
                String::new(),
            ] {
                assert!(
                    TextKind::Topic.check("topic", true, &invalid).is_err(),
                    "case {case} {invalid:?}"
                );
            }
        }
    }
}
