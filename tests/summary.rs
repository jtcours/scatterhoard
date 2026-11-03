//! Labelling the clusters of a set summary by the keywords distinctive to each.
//!
//! The labeller is the half of [`derived-set-summary`](../TRACKING.md) that decides what a cluster
//! is *called*. Measured on this project's own two sets on 2026-09-12, it is the only scheme of
//! three that produced legible labels: cluster representatives are one specific claim rather than a
//! theme, and content-derived terms drown in reasoning vocabulary on a prose-heavy corpus.

use scatterhoard::memory::Keyword;
use scatterhoard::summary::{Cluster, label_clusters};

fn keywords(values: &[&str]) -> Vec<Keyword> {
    values
        .iter()
        .map(|value| Keyword::new(*value).expect("test keyword is valid"))
        .collect()
}

fn named(members: &[&[&str]]) -> Cluster {
    Cluster::new(members.iter().map(|member| keywords(member)).collect())
}

fn labels_of(label: &scatterhoard::summary::ClusterLabel) -> Vec<&str> {
    label.keywords.iter().map(Keyword::as_str).collect()
}

#[test]
fn a_keyword_confined_to_one_cluster_labels_it() {
    let clusters = [
        named(&[&["guidance", "airframe"], &["guidance", "telemetry"]]),
        named(&[&["invoicing", "airframe"], &["invoicing", "telemetry"]]),
    ];

    let labelled = label_clusters(&clusters, 1);

    assert_eq!(labels_of(&labelled[0]), ["guidance"]);
    assert_eq!(labels_of(&labelled[1]), ["invoicing"]);
}

#[test]
fn a_keyword_carried_by_the_whole_set_labels_nothing() {
    // `project` is on every memory, so it separates nothing and must never be offered as a label
    // however often it occurs. This is the failure that makes a summary useless rather than wrong.
    let clusters = [
        named(&[&["project", "guidance"], &["project", "guidance"]]),
        named(&[&["project", "invoicing"], &["project", "invoicing"]]),
    ];

    let labelled = label_clusters(&clusters, 4);

    for label in &labelled {
        assert!(
            !labels_of(label).contains(&"project"),
            "a keyword common to the whole set discriminates nothing: {:?}",
            labels_of(label)
        );
    }
}

#[test]
fn matching_is_caseless_so_one_idea_is_not_split_in_two() {
    // `Keyword` deliberately preserves the spelling it was given, so counting raw strings would
    // make `PID` and `pid` two half-strength keywords that each lose to a weaker rival.
    let clusters = [
        named(&[&["PID"], &["pid"], &["Pid"]]),
        named(&[&["invoicing"], &["invoicing"], &["invoicing"]]),
    ];

    let labelled = label_clusters(&clusters, 4);
    let first = labels_of(&labelled[0]);

    let spellings = first
        .iter()
        .filter(|value| value.eq_ignore_ascii_case("pid"))
        .count();
    assert_eq!(spellings, 1, "one idea, one label: {first:?}");
}

#[test]
fn the_reported_spelling_is_the_one_most_used() {
    let clusters = [
        named(&[&["PID"], &["pid"], &["pid"]]),
        named(&[&["invoicing"], &["invoicing"], &["invoicing"]]),
    ];

    let labelled = label_clusters(&clusters, 4);

    assert_eq!(labels_of(&labelled[0]), ["pid"]);
}

#[test]
fn clusters_are_reported_largest_first() {
    let clusters = [
        named(&[&["small"]]),
        named(&[&["large"], &["large"], &["large"]]),
        named(&[&["medium"], &["medium"]]),
    ];

    let labelled = label_clusters(&clusters, 1);

    let sizes: Vec<usize> = labelled.iter().map(|label| label.size).collect();
    assert_eq!(sizes, [3, 2, 1]);
    assert_eq!(labels_of(&labelled[0]), ["large"]);
}

#[test]
fn a_cluster_whose_keywords_are_all_shared_is_reported_with_no_labels() {
    // Reported, not omitted: a cluster that exists and cannot be named is a fact about the set, and
    // dropping it would make the sizes stop summing to the set.
    let clusters = [
        named(&[&["shared"], &["shared"]]),
        named(&[&["shared"], &["shared"]]),
    ];

    let labelled = label_clusters(&clusters, 3);

    assert_eq!(labelled.len(), 2);
    assert!(labelled.iter().all(|label| label.keywords.is_empty()));
    assert_eq!(labelled.iter().map(|label| label.size).sum::<usize>(), 4);
}

#[test]
fn a_memory_with_no_keywords_still_counts_toward_the_size() {
    let clusters = [named(&[&["guidance"], &[], &[]]), named(&[&["invoicing"]])];

    let labelled = label_clusters(&clusters, 1);

    assert_eq!(labelled[0].size, 3);
    assert_eq!(labels_of(&labelled[0]), ["guidance"]);
}

// ---------------------------------------------------------------------------
// Clustering: grouping a set's memories by what they are about.
// ---------------------------------------------------------------------------

use scatterhoard::embedding::Vector;
use scatterhoard::memory::MemoryId;
use scatterhoard::summary::{Subject, cluster, clusters_for, summarise};

/// A memory with a fixed id, so the keys seeding hashes from ids are the same on every run.
fn subject(number: u32, components: &[f32], words: &[&str]) -> Subject {
    Subject::new(
        &MemoryId::parse(&format!("{number:026}")).expect("a fixed test id"),
        Vector::new(components.to_vec()).expect("test vector is valid"),
        keywords(words),
    )
}

/// Two well-separated groups, deliberately unequal in size so the ordering is observable.
fn two_groups() -> Vec<Subject> {
    vec![
        subject(1, &[1.0, 0.0, 0.0], &["guidance"]),
        subject(2, &[0.98, 0.05, 0.0], &["guidance"]),
        subject(3, &[0.95, 0.10, 0.0], &["guidance"]),
        subject(4, &[0.0, 1.0, 0.0], &["invoicing"]),
        subject(5, &[0.05, 0.98, 0.0], &["invoicing"]),
    ]
}

/// Memories scattered with no group structure at all, so there are many near-equal ways to partition
/// them and which one wins is decided by the seeds. Each memory carries a keyword naming it, so a
/// cluster's membership can be read back.
///
/// **Structureless on purpose.** The first version of this fixture had five loose groups, and the
/// ten restarts found the same best partition from any seeds, so it could not tell seeding by
/// position from seeding by id: a mutation putting positional draws back survived it. Only where the
/// candidates are close does the choice of seeds decide the outcome.
fn crowded() -> Vec<Subject> {
    let mut state: u32 = 12_345;
    let mut noise = move || {
        state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
        f32::from(u16::try_from(state >> 16).expect("fits")) / 65_536.0 - 0.5
    };
    (1..=60)
        .map(|number| {
            let components: Vec<f32> = (0..6).map(|_| noise()).collect();
            subject(number, &components, &[&format!("memory-{number}")])
        })
        .collect()
}

/// A partition as a set of sets, so two partitions of the same memories compare equal whatever order
/// the memories arrived in.
fn membership(clusters: &[Cluster]) -> Vec<Vec<String>> {
    let mut groups: Vec<Vec<String>> = clusters
        .iter()
        .map(|part| {
            let mut names: Vec<String> = part
                .members()
                .iter()
                .flatten()
                .map(|keyword| keyword.as_str().to_owned())
                .collect();
            names.sort();
            names
        })
        .collect();
    groups.sort();
    groups
}

/// **The order memories arrive in must not decide how they are grouped.** A set's memories are read
/// in id order, so inserting one shifts the position of every memory after it; seeding that depended
/// on position regrouped most of a set whenever two memories were added —
/// `a-set-description-reshuffles-on-a-small-change`. Reversing the input is the sharpest version of
/// that shift: every memory moves, and nothing about the memories changes.
#[test]
fn the_order_memories_arrive_in_does_not_change_how_they_are_grouped() {
    let forwards = crowded();
    let mut backwards = crowded();
    backwards.reverse();

    assert_eq!(
        membership(&cluster(&forwards, 3)),
        membership(&cluster(&backwards, 3)),
        "the same memories were grouped differently because they arrived in a different order"
    );
}

#[test]
fn separated_groups_come_out_as_separate_clusters() {
    let clustered = cluster(&two_groups(), 2);

    let mut sizes: Vec<usize> = clustered.iter().map(Cluster::size).collect();
    sizes.sort_unstable();
    assert_eq!(sizes, [2, 3]);
}

#[test]
fn every_memory_lands_in_exactly_one_cluster() {
    let subjects = two_groups();
    let clustered = cluster(&subjects, 2);

    let placed: usize = clustered.iter().map(Cluster::size).sum();
    assert_eq!(placed, subjects.len(), "a partition loses nothing");
}

#[test]
fn clustering_is_deterministic() {
    // Reproducibility is not a nicety here: the summary is written into the exported file and
    // recomputed at import, and the two are compared. A seeded or hash-ordered initialisation would
    // make that comparison disagree with itself on the same data.
    let subjects = two_groups();

    assert_eq!(cluster(&subjects, 2), cluster(&subjects, 2));
}

#[test]
fn asking_for_more_clusters_than_memories_yields_no_empty_clusters() {
    let subjects = two_groups();

    let clustered = cluster(&subjects, 99);

    assert!(clustered.iter().all(|part| part.size() > 0));
    assert_eq!(
        clustered.iter().map(Cluster::size).sum::<usize>(),
        subjects.len()
    );
}

#[test]
fn a_set_with_nothing_in_it_has_no_clusters() {
    assert!(cluster(&[], 4).is_empty());
    assert_eq!(clusters_for(0), 0);
}

#[test]
fn the_cluster_count_is_calibrated_to_the_two_sets_it_was_judged_on() {
    // 8 for `scatterhoard` and 6 for `coding-lessons` are the values whose output was read on
    // 2026-09-12 and found legible. They are the only evidence the rule has, so they are pinned.
    assert_eq!(clusters_for(829), 8);
    assert_eq!(clusters_for(135), 6);
}

#[test]
fn a_set_too_small_to_partition_is_left_whole() {
    assert_eq!(clusters_for(1), 1);
    assert_eq!(clusters_for(3), 1);
}

#[test]
fn the_cluster_count_never_runs_away_with_the_set_size() {
    assert!(
        clusters_for(1_000_000) <= 12,
        "a summary has to stay readable"
    );
}

#[test]
fn summarise_names_each_group_by_what_separates_it() {
    let labelled = summarise(&two_groups(), 1);

    assert_eq!(labelled.len(), 2);
    assert_eq!(labels_of(&labelled[0]), ["guidance"], "largest first");
    assert_eq!(labels_of(&labelled[1]), ["invoicing"]);
}

/// **A fingerprint changes with anything a partition reads**, so a partition kept beside one is
/// recomputed when, and only when, it could come out differently:
/// `each-review-page-recomputes-the-whole-set`.
#[test]
fn a_fingerprint_changes_with_every_input_a_partition_reads() {
    use scatterhoard::summary::fingerprint;
    let held = fingerprint(&two_groups());
    assert_eq!(
        fingerprint(&two_groups()),
        held,
        "the same input, the same print"
    );

    let mut moved = two_groups();
    moved[4] = subject(5, &[0.05, 0.97, 0.0], &["invoicing"]);
    let mut renamed = two_groups();
    renamed[4] = subject(5, &[0.05, 0.98, 0.0], &["receiving"]);
    let mut regrouped = two_groups();
    regrouped[4] = subject(5, &[0.05, 0.98, 0.0], &["invoicing", "guidance"]);
    let mut replaced = two_groups();
    replaced[4] = subject(6, &[0.05, 0.98, 0.0], &["invoicing"]);
    let mut shorter = two_groups();
    shorter.pop();
    let joined = vec![subject(1, &[1.0, 0.0, 0.0], &["guidance"])];
    let split = vec![subject(1, &[1.0, 0.0, 0.0], &["guid", "ance"])];
    assert_ne!(fingerprint(&split), fingerprint(&joined), "a keyword split");
    for (what, changed) in [
        ("a vector", moved),
        ("a keyword", renamed),
        ("a keyword added", regrouped),
        ("an id", replaced),
        ("a memory dropped", shorter),
    ] {
        assert_ne!(fingerprint(&changed), held, "{what}");
    }
}

// ---------------------------------------------------------------------------
// The summary as it is shown: derived whenever a set is described, never stored.
// ---------------------------------------------------------------------------

use scatterhoard::summary::{DERIVATION, SetSummary};

#[test]
fn a_summary_records_the_derivation_that_produced_it() {
    assert_eq!(SetSummary::derive(&two_groups(), 3).derivation, DERIVATION);
}
