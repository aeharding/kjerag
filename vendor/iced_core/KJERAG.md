# Local iced_core patch

Source: `iced/core` in libcosmic commit
`dc1cf9f00cbe2902a52166492654bb9fee8a73d1`, including its iced MIT license.
The local change is in `src/widget/tree.rs`. The manifest expands that
workspace's inherited values, keeps sibling packages at the exact same git
revision and disables publishing.

The pinned child-tree reconciliation matched named children by identity but
left a match in its old vector slot. Inserting a new named child before an
existing one therefore diffed the survivor in the old slot and then overwrote
it with the insertion. The resulting state tree was shorter than the widget
list until another application update rebuilt it.

Kjerag exposed this when pointer motion restored its optional COSMIC header
before the named content container. The malformed rebuild contained only the
header, so the video Scene received no redraw event and could not pump the
presentation clock. The controls' 250 ms timer caused another rebuild and
recreated the missing content, matching the measured playback pause.

The patch takes ownership of the old children and reconstructs the result in
new widget order. It preserves the existing policy: custom IDs match by name,
and every other child is reused only at the same index. It also postpones
drops until matching is complete, so removing an earlier child cannot truncate
a later named survivor before it is found.

The regressions live in `crates/app/src/controls_tree_tests.rs`, where they use
this patched dependency through Kjerag's ordinary workspace dependency graph.
They cover named insertion, removal and reordering, mixed named and positional
children, and replacement when a matching name changes widget tag.

Remove this patch when the pinned iced child reconciliation both places named
survivors in new widget order and retains them across insertions and removals.
Do not send it or an issue to an outside project.
