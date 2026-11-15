//! A merge stage or a blob is read by line range through a pipe into `sed -n`, `head` or `tail`;
//! a redirect to a file stays refused, with a gotcha that names the piped form.

use ra_gotchas::shape::{Shape, Shell, judge};

fn bash(command: &str) -> Shape {
    let dir = tempfile::tempdir().expect("a tempdir");
    judge(command, Shell::Bash, dir.path(), &|_| false)
}

fn refused_by(command: &str) -> Option<String> {
    ra_gotchas::Catalogue::seed().matching(command).map(|g| g.id.clone())
}

#[test]
fn a_stage_read_through_a_pipe_passes_and_a_redirect_is_refused_with_the_piped_form() {
    let reads = [
        "git show :2:crates/ra-console/src/lib.rs | sed -n '1230,1245p'",
        "git show :3:apps/rc/src/main.rs | head -n 40",
        "git show :1:apps/rc/src/main.rs | tail -n 20",
        "git cat-file -p abc1234 | head | tail",
        "git cat-file -p abc1234 | sed -n '5,9p'",
        "git show :2:apps/rc/src/main.rs | head -n 80 | tail -n 10",
        "git show HEAD:apps/rc/src/main.rs | sed -n '10,20p'",
        "git show :2:apps/rc/src/main.rs | sed -n '5p'",
        "git show :2:apps/rc/src/main.rs | sed -n '/fn main/,/^}/p'",
        "git cat-file -p HEAD:apps/rc/src/main.rs | tail -n 5",
        "git cat-file -p abc1234 | head -n 3",
        "git show :3:apps/rc/src/main.rs | head",
    ];
    for read in reads {
        assert_eq!(bash(read), Shape::Allowed, "{read}");
    }
    // the commands a worker resolving a conflict was refused:
    // a merge stage by line range, and a blob through
    // two filters
    assert_eq!(bash("git show :2:apps/rc/src/main.rs | sed -n '1230,1245p'"), Shape::Allowed);
    assert_eq!(bash("git cat-file -p abc1234 | head | tail"), Shape::Allowed);
    // a redirect is no read: it stays refused, and the gotcha
    // names the piped form that reads the same range
    // without a file
    let redirect = "git show :2:apps/rc/src/main.rs > /tmp/ours.rs";
    assert!(matches!(bash(redirect), Shape::Denied(_)));
    assert_eq!(refused_by(redirect).as_deref(), Some("git-stage-redirect"));
    assert_eq!(refused_by("git cat-file -p abc1234 > ../ours.rs").as_deref(), Some("git-stage-redirect"));
    assert_eq!(refused_by("git show :2:a.rs | sed -n '1,5p'"), None);
    let message = ra_gotchas::Catalogue::seed().get("git-stage-redirect").map(|g| g.message.clone()).unwrap_or_default();
    assert!(message.contains("git show :2:<path> | sed -n"), "{message}");
}
