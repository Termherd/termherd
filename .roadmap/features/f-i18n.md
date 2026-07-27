+++
id = "F-i18n"
type = "feature"
area = ["workspace"]
status = "todo"
target = ["Backlog"]
+++

Internationalization — parked; the English-first externalization shipped.

Internationalization. **Parked** (feature-torture ⏸ `F-i18n.md`): heaviest,
least urgent. The pressure test surfaced a *present* issue though — the UI was
hardcoded **French** in an English-README repo, with no string externalization.
Canonical UI language settled as **English-first**; the externalization
precursor shipped (#60), centralising every user-facing string in
`crates/app/src/strings.rs`. Locale machinery (catalogues, selection,
width/RTL) stays parked until a non-English user base appears
