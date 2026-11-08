//! The one sentence each insights page opens with: the finding, computed from the same numbers
//! the page shows below it, never a canned line. A page with nothing counted gets no sentence.

use crate::fmt::{count, noun, thousands};

/// Activity: how much happened in the period and how it compares with the one before.
pub(crate) fn activity(commits: u64, authors: u64, before: Option<u64>) -> String {
    if commits == 0 {
        return String::new();
    }
    let what = format!(
        "{} by {}",
        count(commits, "commit", "commits"),
        count(authors, "author", "authors")
    );
    match before {
        None => format!("{what}."),
        Some(0) => format!("{what}, after none in the period before."),
        Some(b) if b == commits => format!("{what}, as many as in the period before."),
        Some(b) => {
            let pct = (commits as f64 - b as f64) / b as f64 * 100.0;
            // A change that rounds to 0% is no change worth a number.
            if pct.abs() < 0.5 {
                return format!("{what}, about as many as in the period before.");
            }
            let dir = if pct > 0.0 { "more" } else { "fewer" };
            format!(
                "{what}, {:.0}% {dir} than the period before.",
                pct.abs().round()
            )
        }
    }
}

/// Contributors: how many people, and how many of them are new.
pub(crate) fn contributors(total: usize, new: usize) -> String {
    if total == 0 {
        return String::new();
    }
    let people = count(total, "contributor", "contributors");
    match new {
        0 => format!("{people}, none of them new in this period."),
        n if n == total => format!("{people}, all of them new in this period."),
        n => format!("{people}, {} of them new in this period.", thousands(n)),
    }
}

/// Hotspots: how concentrated change is.
pub(crate) fn hotspots(files: usize, concentration: f64) -> String {
    if files == 0 {
        return String::new();
    }
    format!(
        "A tenth of the {} takes {:.0}% of all changes.",
        count(files, "changed file", "changed files"),
        concentration * 100.0
    )
}

/// Ownership: how few people hold half of the code, and where one person holds it alone.
/// `owners` counts everyone with a share of the code the page shows; when that is one person,
/// the sentence says they wrote it all instead of the odd "1 person holds half".
pub(crate) fn ownership(bus_factor: usize, owners: usize, dirs_at_one: usize) -> String {
    if bus_factor == 0 {
        return String::new();
    }
    let hold = if owners <= 1 {
        "One person wrote all of the code".to_owned()
    } else {
        format!(
            "{} {} half of the code",
            count(bus_factor, "person", "people"),
            noun(bus_factor, "holds", "hold")
        )
    };
    let dirs = match dirs_at_one {
        0 => "No directory has a single owner.".to_owned(),
        1 => "1 directory has a single owner.".to_owned(),
        n => format!("{} directories have a single owner.", thousands(n)),
    };
    format!("{hold}. {dirs}")
}

/// Languages: the largest language and its share of all lines.
pub(crate) fn languages(top: &str, share: &str, total: i64) -> String {
    if total <= 0 || top.is_empty() {
        return String::new();
    }
    format!("{top} is {share} of {}.", count(total, "line", "lines"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn activity_compares_with_the_period_before() {
        assert_eq!(
            activity(12, 3, Some(10)),
            "12 commits by 3 authors, 20% more than the period before."
        );
        assert_eq!(
            activity(5, 1, Some(10)),
            "5 commits by 1 author, 50% fewer than the period before."
        );
        assert_eq!(
            activity(4, 2, Some(4)),
            "4 commits by 2 authors, as many as in the period before."
        );
        // 1,001 against 1,000 is 0.1% more: never "0% more".
        assert_eq!(
            activity(1001, 2, Some(1000)),
            "1,001 commits by 2 authors, about as many as in the period before."
        );
        assert_eq!(activity(1, 1, None), "1 commit by 1 author.");
        assert_eq!(activity(0, 0, Some(3)), "");
    }

    #[test]
    fn contributors_name_the_new_ones() {
        assert_eq!(
            contributors(1200, 3),
            "1,200 contributors, 3 of them new in this period."
        );
        assert_eq!(
            contributors(2, 2),
            "2 contributors, all of them new in this period."
        );
        assert_eq!(contributors(0, 0), "");
    }

    #[test]
    fn hotspots_state_the_concentration() {
        assert_eq!(
            hotspots(25, 0.62),
            "A tenth of the 25 changed files takes 62% of all changes."
        );
        assert_eq!(hotspots(0, 0.0), "");
    }

    #[test]
    fn ownership_counts_people_and_lone_directories() {
        assert_eq!(
            ownership(3, 9, 25),
            "3 people hold half of the code. 25 directories have a single owner."
        );
        assert_eq!(
            ownership(1, 4, 1),
            "1 person holds half of the code. 1 directory has a single owner."
        );
        assert_eq!(ownership(0, 0, 0), "");
    }

    #[test]
    fn a_single_author_wrote_all_of_the_code() {
        assert_eq!(
            ownership(1, 1, 3),
            "One person wrote all of the code. 3 directories have a single owner."
        );
    }

    #[test]
    fn languages_name_the_largest() {
        assert_eq!(
            languages("Rust", "62.0%", 66_500),
            "Rust is 62.0% of 66,500 lines."
        );
        assert_eq!(languages("", "", 0), "");
    }
}
