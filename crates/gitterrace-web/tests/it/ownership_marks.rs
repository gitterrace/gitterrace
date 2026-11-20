//! The ownership table marks a bus factor of 1 in the warning color, not the error red, leaves
//! 2 and up plain, and never lists an owner whose share rounds to 0%.

use gitterrace_testkit::{Fixture, page};

#[tokio::test]
async fn lone_owners_warn_and_empty_shares_are_left_out() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut f = Fixture::init(dir.path());
    // Ann writes almost all of solo/, both share shared/ and Bob touches solo/ once, lightly.
    let big = "x\n".repeat(400);
    f = f.write("solo/a.txt", &big).write("shared/b.txt", "a\n");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-q", "--author", "Ann <ann@x.io>", "-m", "feat: ann"]);
    f = f.write("shared/b.txt", &"b\n".repeat(50)).write("solo/c.txt", "c\n");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-q", "--author", "Bob <bob@x.io>", "-m", "feat: bob"]);
    f = f.write("shared/d.txt", &"d\n".repeat(50));
    f.git(&["add", "-A"]);
    f.git(&["commit", "-q", "--author", "Ann <ann@x.io>", "-m", "feat: ann again"]);
    let router = crate::common::ready_router(f.path()).await;
    let (status, body) = page(&router, "/stats/ownership").await;
    assert_eq!(status, 200);
    let table = body.split("class=\"entries own\"").nth(1).expect("table");
    let table = table.split("</table>").next().unwrap_or_default();
    assert!(!table.contains("class=\"del\""), "bus factor shown as an error");
    assert!(!body.contains("data-solo"), "two owners named as one");
    assert!(table.contains("<th scope=\"col\">Owners</th>"), "the Owners column is gone");
    for row in table.split("<tr>").skip(2) {
        let bus = row
            .split("<td class=\"r bus\">")
            .nth(1)
            .and_then(|c| c.split("</td>").next())
            .unwrap_or_default();
        if bus.contains(">1</b>") {
            assert!(bus.contains("class=\"warn\""), "a lone owner without the warning: {row}");
        } else {
            assert!(!bus.contains("<b"), "a bus factor of 2 or more is marked: {row}");
        }
        assert!(!row.contains(">0%<"), "an owner at 0% is listed: {row}");
    }
}

#[tokio::test]
async fn one_owner_everywhere_drops_the_owner_columns() {
    let dir = tempfile::tempdir().expect("tmp");
    let mut f = Fixture::init(dir.path());
    f = f.write("src/a.txt", "a\n").write("docs/b.txt", "b\n");
    f.git(&["add", "-A"]);
    f.git(&["commit", "-q", "--author", "Ann <ann@x.io>", "-m", "feat: ann"]);
    let router = crate::common::ready_router(f.path()).await;
    let (status, body) = page(&router, "/stats/ownership").await;
    assert_eq!(status, 200);
    assert!(body.contains("Every directory has one owner, Ann."), "no single-owner line");
    let table = body.split("class=\"entries own\"").nth(1).expect("table");
    let table = table.split("</table>").next().unwrap_or_default();
    assert!(!table.contains(">Owners</th>"), "the Owners column is shown");
    assert!(!table.contains("Bus factor"), "the Bus factor column is shown");
    let kpis = body.split("class=\"kpis\"").nth(1).expect("kpis");
    assert!(
        kpis.contains("<b class=\"warn\">1</b><span>Bus factor</span>"),
        "a bus factor of 1 is not in the warning color"
    );
}
