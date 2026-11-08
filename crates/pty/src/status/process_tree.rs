//! Finding a program among a process's descendants, from a flat process list.
//!
//! ConPTY has no foreground process group, so on Windows the Claude running
//! in a pane is found by walking down from the pane's shell instead. The walk
//! is pure: the platform listing it reads lives in `win_process`, so the
//! choice of descendant is tested on every OS.

use std::collections::{HashMap, HashSet, VecDeque};

/// One entry of a process listing: who it is, who started it, and its image
/// name (`claude.exe`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Process {
    pub(crate) pid: u32,
    pub(crate) parent: u32,
    pub(crate) name: String,
}

/// Each process's children, in listing order. A process naming itself as its
/// parent is nobody's child.
fn children(processes: &[Process]) -> HashMap<u32, Vec<&Process>> {
    let mut children: HashMap<u32, Vec<&Process>> = HashMap::new();
    for process in processes
        .iter()
        .filter(|process| process.pid != process.parent)
    {
        children.entry(process.parent).or_default().push(process);
    }
    children
}

/// The nearest descendant of `root` whose image name is `name` (compared
/// without case, as Windows does), or `None` when there is none.
///
/// Nearest means fewest generations down, so a `claude` that Claude itself
/// starts never wins over it; between two at the same depth, the earlier one
/// in the listing does. A listing can hold a cycle — Windows reuses a dead
/// parent's pid, so a child can name a younger process as its parent — and a
/// process seen once is not walked again.
fn nearest_descendant(
    children: &HashMap<u32, Vec<&Process>>,
    root: u32,
    name: &str,
) -> Option<u32> {
    let mut seen = HashSet::from([root]);
    let mut queue = VecDeque::from([root]);
    while let Some(pid) = queue.pop_front() {
        for child in children.get(&pid).into_iter().flatten() {
            if !seen.insert(child.pid) {
                continue;
            }
            if child.name.eq_ignore_ascii_case(name) {
                return Some(child.pid);
            }
            queue.push_back(child.pid);
        }
    }
    None
}

/// The job running in front of `shell`, as ConPTY lets it be read: the
/// nearest descendant named `claude` — the pid its session file is named
/// after — and otherwise the shell's first child, so a program that is not
/// Claude still reads as running. `None` while the shell runs nothing.
pub(crate) fn job_under(processes: &[Process], shell: u32, claude: &str) -> Option<u32> {
    let children = children(processes);
    nearest_descendant(&children, shell, claude)
        .or_else(|| Some(children.get(&shell)?.first()?.pid))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHELL: u32 = 100;

    fn process(pid: u32, parent: u32, name: &str) -> Process {
        Process {
            pid,
            parent,
            name: name.to_owned(),
        }
    }

    fn claude_under_shell(processes: &[Process]) -> Option<u32> {
        nearest_descendant(&children(processes), SHELL, "claude.exe")
    }

    #[test]
    fn a_claude_started_by_the_shell_is_found() {
        let processes = [
            process(1, 0, "termherd.exe"),
            process(SHELL, 1, "pwsh.exe"),
            process(201, SHELL, "claude.exe"),
        ];
        assert_eq!(claude_under_shell(&processes), Some(201));
    }

    #[test]
    fn the_image_name_is_matched_without_case() {
        let processes = [process(201, SHELL, "Claude.EXE")];
        assert_eq!(claude_under_shell(&processes), Some(201));
    }

    #[test]
    fn a_claude_below_an_intermediate_process_is_found() {
        let processes = [
            process(150, SHELL, "cmd.exe"),
            process(202, 150, "claude.exe"),
        ];
        assert_eq!(claude_under_shell(&processes), Some(202));
    }

    #[test]
    fn a_claude_under_another_shell_is_not_this_panes() {
        let processes = [
            process(SHELL, 1, "pwsh.exe"),
            process(300, 1, "pwsh.exe"),
            process(301, 300, "claude.exe"),
        ];
        assert_eq!(claude_under_shell(&processes), None);
    }

    #[test]
    fn the_outermost_claude_wins_over_one_it_started() {
        // Listed child first, so the depth decides, not the listing order.
        let processes = [
            process(402, 401, "claude.exe"),
            process(401, SHELL, "claude.exe"),
        ];
        assert_eq!(claude_under_shell(&processes), Some(401));
    }

    #[test]
    fn between_two_claudes_at_one_depth_the_earlier_listed_wins() {
        let processes = [
            process(502, SHELL, "claude.exe"),
            process(501, SHELL, "claude.exe"),
        ];
        assert_eq!(claude_under_shell(&processes), Some(502));
    }

    #[test]
    fn a_shell_running_nothing_named_claude_yields_none() {
        let processes = [
            process(SHELL, 1, "pwsh.exe"),
            process(601, SHELL, "git.exe"),
        ];
        assert_eq!(claude_under_shell(&processes), None);
    }

    #[test]
    fn a_parent_cycle_from_a_reused_pid_ends_the_walk() {
        // 701 names 702 as its parent and 702 names 701: a reused pid.
        let processes = [
            process(701, SHELL, "node.exe"),
            process(702, 701, "node.exe"),
            process(701, 702, "node.exe"),
            process(703, 703, "claude.exe"),
        ];
        assert_eq!(claude_under_shell(&processes), None);
    }

    #[test]
    fn a_job_under_the_shell_is_its_claude_when_one_runs() {
        let processes = [
            process(801, SHELL, "git.exe"),
            process(802, SHELL, "cmd.exe"),
            process(803, 802, "claude.exe"),
        ];
        assert_eq!(job_under(&processes, SHELL, "claude.exe"), Some(803));
    }

    #[test]
    fn a_job_under_the_shell_is_its_first_child_when_no_claude_runs() {
        let processes = [
            process(SHELL, 1, "pwsh.exe"),
            process(901, SHELL, "vim.exe"),
            process(902, SHELL, "git.exe"),
        ];
        assert_eq!(job_under(&processes, SHELL, "claude.exe"), Some(901));
    }

    #[test]
    fn a_shell_at_its_prompt_has_no_job() {
        let processes = [
            process(SHELL, 1, "pwsh.exe"),
            process(SHELL, SHELL, "x.exe"),
        ];
        assert_eq!(job_under(&processes, SHELL, "claude.exe"), None);
    }

    #[test]
    fn a_process_naming_itself_as_parent_is_not_its_own_descendant() {
        let processes = [process(SHELL, SHELL, "claude.exe")];
        assert_eq!(claude_under_shell(&processes), None);
    }
}
