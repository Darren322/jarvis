use super::detail_matches_fact;

const SISTER: &str = "The user has a sister named Mia.";

#[rustfmt::skip]
#[test]
fn a_detail_matches_when_each_meaningful_word_starts_a_word_of_the_fact() {
    let cases: &[(&str, &str, bool)] = &[
        // The transcribed sentence from the Pi.
        ("that my sister name is Mia.", SISTER, true),
        ("my sister name is Maya please", SISTER, false),
        // Prefixes, case and punctuation.
        ("name", SISTER, true),
        ("sis", SISTER, true),
        ("MIA!", SISTER, true),
        ("mia's sister", SISTER, true),
        ("sister's mia", "The user's sister is called Mia.", true),
        ("naming", SISTER, false),
        // A word must start a fact word, not sit inside one.
        ("ister", SISTER, false),
        // Every word must match.
        ("sister brother", SISTER, false),
        // Filler words are dropped, so they neither help nor hurt.
        ("that the sister of me", SISTER, true),
        ("called", "The user lives in Leeds.", false),
        // Nothing left after the filler: no match.
        ("that", SISTER, false),
        ("that my is a please", SISTER, false),
        ("", SISTER, false),
        ("...", SISTER, false),
        // Unmatched details.
        ("parrot", SISTER, false),
    ];
    for (detail, fact, expected) in cases {
        assert_eq!(detail_matches_fact(detail, fact), *expected, "{detail:?} vs {fact:?}");
    }
}
