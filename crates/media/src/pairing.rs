//! One frame-index alignment rule for GPU and system-memory deliveries.

/// What the current head of every decoded lens queue permits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Alignment {
    /// At least one lane has no head yet. Do not discard any other lane.
    Waiting,
    /// Every lane names this frame index.
    Ready(u64),
    /// Advance only heads older than this newest head, then check again.
    DropBefore(u64),
}

/// Decide alignment without owning the queues or their delivery policy.
/// Callers convert source-local timestamps to capture indices first.
pub(crate) fn alignment(heads: impl IntoIterator<Item = Option<u64>>) -> Alignment {
    let mut heads = heads.into_iter();
    let Some(Some(first)) = heads.next() else {
        return Alignment::Waiting;
    };
    let mut newest = first;
    let mut equal = true;
    for head in heads {
        let Some(index) = head else {
            return Alignment::Waiting;
        };
        newest = newest.max(index);
        equal &= index == first;
    }
    if equal {
        Alignment::Ready(first)
    } else {
        Alignment::DropBefore(newest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_lanes_or_a_missing_head_waits() {
        assert_eq!(alignment([]), Alignment::Waiting);
        assert_eq!(alignment([None]), Alignment::Waiting);
        assert_eq!(alignment([Some(4), None]), Alignment::Waiting);
        assert_eq!(alignment([None, Some(4)]), Alignment::Waiting);
        // An earlier mismatch is not permission to drop before all heads exist.
        assert_eq!(alignment([Some(4), Some(7), None]), Alignment::Waiting);
    }

    #[test]
    fn equal_heads_are_ready_and_only_older_heads_may_advance() {
        assert_eq!(alignment([Some(4)]), Alignment::Ready(4));
        assert_eq!(alignment([Some(4), Some(4)]), Alignment::Ready(4));
        assert_eq!(
            alignment([Some(7), Some(4), Some(7)]),
            Alignment::DropBefore(7)
        );
    }

    #[test]
    fn comparison_never_loses_integer_precision() {
        for index in [(1_u64 << 53) + 1, u64::MAX] {
            assert_eq!(
                alignment([Some(index), Some(index)]),
                Alignment::Ready(index)
            );
            assert_eq!(
                alignment([Some(index - 1), Some(index)]),
                Alignment::DropBefore(index)
            );
        }
    }

    #[test]
    fn agrees_with_the_readers_previous_head_decision() {
        // Exhaust the short cases, including absent heads, rather than checking
        // only a favorable pair. Reader retains its separate depth/error rules.
        let heads = [None, Some(0), Some(1), Some(2), Some(3)];
        for first in heads {
            for second in heads {
                for third in heads {
                    let input = [first, second, third];
                    let present: Vec<u64> = input.iter().flatten().copied().collect();
                    let expected = if present.len() != input.len() {
                        Alignment::Waiting
                    } else {
                        let newest = *present.iter().max().unwrap();
                        if present.iter().all(|index| *index == newest) {
                            Alignment::Ready(newest)
                        } else {
                            Alignment::DropBefore(newest)
                        }
                    };
                    assert_eq!(alignment(input), expected, "heads: {input:?}");
                }
            }
        }
    }
}
