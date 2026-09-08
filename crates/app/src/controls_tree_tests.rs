use cosmic::iced::advanced::widget::{
    Id, Tree,
    tree::{State, Tag},
};

struct FirstTag;
struct SecondTag;

struct Child {
    id: Option<Id>,
    tag: Tag,
    fresh: u8,
}

fn named(name: &'static str, tag: Tag, fresh: u8) -> Child {
    Child {
        id: Some(Id::new(name)),
        tag,
        fresh,
    }
}

fn unnamed(tag: Tag, fresh: u8) -> Child {
    Child {
        id: None,
        tag,
        fresh,
    }
}

fn tree(child: &Child) -> Tree {
    Tree {
        tag: child.tag,
        id: child.id.clone(),
        state: State::new(child.fresh),
        children: Vec::new(),
    }
}

fn reconcile(parent: &mut Tree, children: &mut [Child]) {
    parent.diff_children_custom(
        children,
        children.iter().map(|child| child.id.clone()).collect(),
        |state, child| {
            if state.tag != child.tag {
                *state = tree(child);
            }
        },
        tree,
    );
}

fn state(tree: &Tree) -> u8 {
    *tree.state.downcast_ref()
}

fn parent(children: Vec<Tree>) -> Tree {
    Tree {
        tag: Tag::stateless(),
        id: None,
        state: State::None,
        children,
    }
}

#[test]
fn named_insertion_preserves_the_survivor_in_new_order() {
    let tag = Tag::of::<FirstTag>();
    let mut parent = parent(vec![tree(&named("content", tag, 7))]);
    let mut children = [named("header", tag, 10), named("content", tag, 99)];

    reconcile(&mut parent, &mut children);

    assert_eq!(parent.children.len(), 2);
    assert_eq!(
        parent.children[0]
            .id
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("header")
    );
    assert_eq!(
        parent.children[1]
            .id
            .as_ref()
            .map(ToString::to_string)
            .as_deref(),
        Some("content")
    );
    assert_eq!(
        (state(&parent.children[0]), state(&parent.children[1])),
        (10, 7)
    );
}

#[test]
fn named_removal_does_not_truncate_a_later_survivor() {
    let tag = Tag::of::<FirstTag>();
    let mut parent = parent(vec![
        tree(&named("header", tag, 10)),
        tree(&named("content", tag, 7)),
    ]);
    let mut children = [named("content", tag, 99)];

    reconcile(&mut parent, &mut children);

    assert_eq!(parent.children.len(), 1);
    assert_eq!(state(&parent.children[0]), 7);
}

#[test]
fn named_reordering_moves_state_with_identity() {
    let tag = Tag::of::<FirstTag>();
    let mut parent = parent(vec![
        tree(&named("first", tag, 1)),
        tree(&named("second", tag, 2)),
    ]);
    let mut children = [named("second", tag, 20), named("first", tag, 10)];

    reconcile(&mut parent, &mut children);

    assert_eq!(
        (state(&parent.children[0]), state(&parent.children[1])),
        (2, 1)
    );
}

#[test]
fn unnamed_children_remain_positional_among_named_moves() {
    let tag = Tag::of::<FirstTag>();
    let mut parent = parent(vec![
        tree(&named("first", tag, 1)),
        tree(&unnamed(tag, 2)),
        tree(&named("second", tag, 3)),
    ]);
    let mut children = [
        named("second", tag, 30),
        unnamed(tag, 20),
        named("first", tag, 10),
    ];

    reconcile(&mut parent, &mut children);

    assert_eq!(
        parent.children.iter().map(state).collect::<Vec<_>>(),
        [3, 2, 1]
    );
}

#[test]
fn a_matching_name_with_a_new_tag_rebuilds_state() {
    let first = Tag::of::<FirstTag>();
    let second = Tag::of::<SecondTag>();
    let mut parent = parent(vec![tree(&named("content", first, 7))]);
    let mut children = [named("content", second, 99)];

    reconcile(&mut parent, &mut children);

    assert_eq!(parent.children[0].tag, second);
    assert_eq!(state(&parent.children[0]), 99);
}
