//! A pipeline whose first stage reads a workspace file and whose every stage is a read-only
//! filter passes the hook; any other pipeline stays refused and matches `piped-or-chained`.

use ra_gotchas::shape::{Shape, Shell, judge};

fn bash(command: &str) -> Shape {
    let dir = tempfile::tempdir().expect("a tempdir");
    judge(command, Shell::Bash, dir.path(), &|_| false)
}

fn refused_by(command: &str) -> Option<String> {
    ra_gotchas::Catalogue::seed().matching(command).map(|g| g.id.clone())
}

#[test]
fn a_pipeline_of_read_only_filters_over_a_workspace_file_passes() {
    let reads = [
        "grep -n fn apps/rc/src/main.rs | head -n 5",
        "cat apps/rc/src/main.rs | grep -c fn | wc -l",
        "sed -n '1,40p' apps/rc/src/main.rs | grep use | sort | uniq",
        "grep -n x f | awk -F: '$1>6540 && $1<6620' | tail -n 3",
        "awk '{print $1}' f | sort | uniq | wc -l",
        "sort f | uniq | head -n 4",
        "tail -n 50 f | sed -n '/fn main/,/^}/p' | wc -l",
    ];
    for read in reads {
        assert_eq!(bash(read), Shape::Allowed, "{read}");
    }
}

#[test]
fn a_pipeline_with_a_writer_is_refused_by_the_piped_gotcha() {
    let refused = [
        "grep x f | awk '{print $1 > \"out\"}'",
        "grep x f | awk 'BEGIN{system(\"rm f\")}'",
        "grep x f | sed -i s/a/b/ g",
        "grep x f | sort > out.txt",
        "grep x f | tee out.txt",
    ];
    for command in refused {
        assert_ne!(bash(command), Shape::Allowed, "{command}");
        assert_eq!(refused_by(&format!("{command} ; ls")).as_deref(), Some("piped-or-chained"), "{command}");
    }
}
