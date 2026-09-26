//! The short session-id is the shortest prefix of a `sessionId` that is unique
//! across all Stores, with a minimum of 8 characters. See ADR 0017.

use std::collections::HashMap;

/// The fewest characters a short session-id shows, even when fewer are unique.
const MIN_LEN: usize = 8;

/// The short session-id of each known Session id. Built once from the ids of
/// every Session in every Store, so a short id never depends on the scope of
/// the command that prints it. An id that was not known when the table was
/// built shows the minimum length.
#[derive(Debug, Default)]
pub struct ShortIds {
    lengths: HashMap<String, usize>,
}

impl ShortIds {
    pub fn from_ids(ids: impl IntoIterator<Item = String>) -> Self {
        let mut ids: Vec<String> = ids.into_iter().collect();
        ids.sort();
        ids.dedup();
        // In sorted order, the id that shares the longest prefix with an id
        // is always one of its two neighbours.
        let lengths = ids
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let before = i.checked_sub(1).map_or(0, |j| common_prefix(id, &ids[j]));
                let after = ids.get(i + 1).map_or(0, |next| common_prefix(id, next));
                let unique = (before.max(after) + 1).min(id.chars().count());
                (id.clone(), unique.max(MIN_LEN))
            })
            .collect();
        Self { lengths }
    }

    /// The short session-id to print for `session_id`.
    pub fn of(&self, session_id: &str) -> String {
        let len = self.lengths.get(session_id).copied().unwrap_or(MIN_LEN);
        session_id.chars().take(len).collect()
    }
}

/// The number of leading characters `a` and `b` share.
fn common_prefix(a: &str, b: &str) -> usize {
    a.chars().zip(b.chars()).take_while(|(x, y)| x == y).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(ids: &[&str]) -> ShortIds {
        ShortIds::from_ids(ids.iter().map(|id| id.to_string()))
    }

    #[test]
    fn ids_that_differ_in_the_first_8_characters_show_8() {
        let ids = table(&[
            "11111111-2222-3333-4444-555555555555",
            "22222222-2222-3333-4444-555555555555",
        ]);
        assert_eq!(ids.of("11111111-2222-3333-4444-555555555555"), "11111111");
        assert_eq!(ids.of("22222222-2222-3333-4444-555555555555"), "22222222");
    }

    #[test]
    fn ids_that_share_a_long_prefix_grow_until_they_differ() {
        let ids = table(&[
            "ses_2a7f9c1d4Xaaaa",
            "ses_2a7f9c1d4Ybbbb",
            "ses_9zzzzzzzzzzzzz",
        ]);
        assert_eq!(ids.of("ses_2a7f9c1d4Xaaaa"), "ses_2a7f9c1d4X");
        assert_eq!(ids.of("ses_2a7f9c1d4Ybbbb"), "ses_2a7f9c1d4Y");
        assert_eq!(ids.of("ses_9zzzzzzzzzzzzz"), "ses_9zzz");
    }

    #[test]
    fn only_the_nearest_neighbour_sets_the_length() {
        let ids = table(&["abcdefgh11", "abcdefgh12", "abcdefgh2"]);
        assert_eq!(ids.of("abcdefgh11"), "abcdefgh11");
        assert_eq!(ids.of("abcdefgh12"), "abcdefgh12");
        assert_eq!(ids.of("abcdefgh2"), "abcdefgh2");
    }

    #[test]
    fn an_id_that_is_a_prefix_of_another_shows_whole() {
        let ids = table(&["abcdefghij", "abcdefghijkl"]);
        assert_eq!(ids.of("abcdefghij"), "abcdefghij");
        assert_eq!(ids.of("abcdefghijkl"), "abcdefghijk");
    }

    #[test]
    fn an_id_seen_twice_is_not_its_own_collision() {
        let ids = table(&["abcdefghij", "abcdefghij", "zzzzzzzzzz"]);
        assert_eq!(ids.of("abcdefghij"), "abcdefgh");
    }

    #[test]
    fn an_unknown_or_short_id_shows_at_most_8() {
        let ids = table(&["abcdefghij"]);
        assert_eq!(ids.of("0123456789"), "01234567");
        assert_eq!(ids.of("abc"), "abc");
    }
}
