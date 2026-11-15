//! A command that chains with `;` and pipes into `head` or `grep` matches `piped-or-chained`,
//! whose text names the single unpiped command form.

use ra_gotchas::{Catalogue, Conditions, Verdict};

#[test]
fn a_chain_piped_into_head_or_grep_names_the_single_unpiped_command() {
    for command in ["curl http://x; curl http://y | head -n 5", "curl http://x; curl http://y | grep ok"] {
        let Verdict::Deny { id, reason } = Catalogue::seed().command(command, Conditions::default()) else {
            panic!("{command} is refused with a hint");
        };
        assert_eq!(id, "piped-or-chained", "{command}");
        assert!(reason.contains("single unpiped command"), "{reason}");
        assert!(reason.contains("each command alone"), "{reason}");
    }
}
