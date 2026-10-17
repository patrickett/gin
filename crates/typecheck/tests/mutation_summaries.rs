mod support;

use parser::query::SourceParseExt;
use support::{check, expect_flaws_with, expect_flaws_with_named, flaw_codes, package};

const TYPES: &str = "Numeric has\n    as_num(self ref) is in Self: return (number is in Self: self)\nCount is in 0 to 255\nCount has Numeric\n    release(self):\n        return\nreplace(previous Count, next Count) Count: return next\nRegion has\n    capacity Count\n    initialized Count and <= capacity\n    initialized_count(self ref) is in Count: return self.initialized.as_num\n    capacity_count(self ref) is in Count: return self.capacity.as_num\n    last(self ref and self.initialized_count > 0) Count: return 0\n    empty(self ref and self.initialized_count = 0) Count: return 0\n    full_capacity(self ref and self.capacity_count = 4) Count: return 0\n    expect_count(self ref, count Count and count = self.initialized_count) Count: return count\n    clear(self mut):\n        self.initialized :: replace(self.initialized, 0)\n        return\n    pop(self mut and self.initialized_count > 0) Count:\n        count: self.initialized.as_num\n        index: count - 1\n        self.initialized :: replace(self.initialized, index as Count)\n        return 0\n    release(self):\n        return\n";

const CASES: &[(&str, &str, &[&str])] = &[
    (
        "clearing_establishes_zero",
        "run:\n    region: Region(4, 2)\n    region.clear\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "decrementing_one_establishes_zero",
        "run:\n    region: Region(4, 1)\n    taken: region.pop\n    taken.release\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "decrementing_twice_tracks_both_changes",
        "run:\n    region: Region(4, 2)\n    first: region.pop\n    first.release\n    second: region.pop\n    second.release\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "a_second_pop_of_one_is_rejected",
        "run:\n    region: Region(4, 1)\n    first: region.pop\n    first.release\n    second: region.pop\n    second.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "a_decrement_preserves_capacity",
        "run:\n    region: Region(4, 2)\n    taken: region.pop\n    taken.release\n    value: region.full_capacity\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "changing_another_receiver_preserves_its_facts",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 1)\n    right.clear\n    value: left.last\n    value.release\n    left.release\n    right.release\n    return\n",
        &[],
    ),
    (
        "a_guard_survives_mutation_of_a_disjoint_parameter",
        "read(left ref Region, right mut Region) Count:\n    when left.initialized_count > 0 then\n        right.clear\n        return left.last\n    return 0\n",
        &[],
    ),
    (
        "nested_aliases_mutate_the_original_owner",
        "run:\n    region: Region(4, 1)\n    alias: mut region\n    nested: mut alias\n    nested.clear\n    value: region.last\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "a_saved_count_remains_the_incoming_value",
        "read(region mut Region and region.initialized_count > 0) Count:\n    before: region.initialized_count\n    value: region.pop\n    value.release\n    next: before - 1\n    return region.expect_count(next as Count)\n",
        &[],
    ),
    (
        "sequential_field_writes_use_the_updated_value",
        "clear(region mut Region and region.initialized_count > 0) Count:\n    count: region.initialized.as_num\n    region.initialized :: replace(region.initialized, (count - 1) as Count)\n    return region.expect_count((count - 1) as Count)\n",
        &[],
    ),
    (
        "wrappers_publish_the_callees_guarantees",
        "clear(region mut Region):\n    region.clear\n    return\nrun:\n    region: Region(4, 1)\n    clear(region)\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "clearing_through_an_alias_updates_the_owner",
        "run:\n    region: Region(4, 1)\n    alias: mut region\n    alias.clear\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "clearing_through_an_alias_rejects_the_owners_stale_proof",
        "run:\n    region: Region(4, 1)\n    alias: mut region\n    alias.clear\n    value: region.last\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "clearing_through_an_alias_preserves_capacity",
        "run:\n    region: Region(4, 1)\n    alias: mut region\n    alias.clear\n    value: region.full_capacity\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "conditional_mutation_does_not_publish_one_branches_value",
        "maybe_clear(region mut Region, flag ref Count):\n    when flag.as_num > 0 then\n        region.clear\n        return\n    return\nrun(flag Count):\n    region: Region(4, 1)\n    maybe_clear(region, flag)\n    value: region.empty\n    value.release\n    region.release\n    flag.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "a_loop_does_not_reuse_an_initial_count_on_later_iterations",
        "run:\n    region: Region(4, 1)\n    while region.initialized_count > 0 do\n        taken: region.pop\n        taken.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "a_loop_retargeting_an_alias_invalidates_the_new_receivers_count",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 1)\n    alias: mut left\n    for index in 0 to 2\n        value: right.last\n        value.release\n        alias :: mut right\n        alias.clear\n    left.release\n    right.release\n    return\n",
        &[
            "type-parameter-refinement-failed",
            "type-parameter-refinement-unproven",
        ],
    ),
    (
        "a_loop_retargeting_an_alias_can_reestablish_a_field_guard",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 1)\n    alias: mut left\n    while right.capacity_count = 4 do\n        value: right.full_capacity\n        value.release\n        alias :: mut right\n        alias.clear\n    left.release\n    right.release\n    return\n",
        &[],
    ),
    (
        "a_loop_mutating_one_field_preserves_an_unchanged_field",
        "run:\n    region: Region(4, 1)\n    for index in 0 to 2\n        value: region.full_capacity\n        value.release\n        region.clear\n    value: region.full_capacity\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "wrappers_do_not_preserve_a_field_the_callee_cannot_summarize",
        "choose(flag ref Count) Count:\n    when flag.as_num > 0 then\n        return 0\n    return 1\nRegion has\n    choose_count(self mut and self.capacity_count = 255, flag ref Count):\n        self.initialized :: replace(self.initialized, choose(flag))\n        return\nchoose_count(region mut Region and region.capacity_count = 255, flag ref Count):\n    region.choose_count(flag)\n    return\nrun(flag Count):\n    region: Region(255, 1)\n    choose_count(region, flag)\n    value: region.last\n    value.release\n    region.release\n    flag.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "mutating_a_distinct_local_does_not_publish_a_mutation_of_the_parameter",
        "clear_local(region mut Region):\n    local: Region(4, 1)\n    local.clear\n    local.release\n    return\nrun:\n    region: Region(4, 1)\n    clear_local(region)\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "reassigning_an_alias_changes_which_owner_is_mutated",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 1)\n    alias: mut left\n    alias :: mut right\n    when right.initialized_count > 0 then\n        alias.clear\n        value: right.last\n        value.release\n        left.release\n        right.release\n        return\n    left.release\n    right.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "later_arguments_cannot_change_an_already_evaluated_count",
        "Region has\n    restore(self mut, next Count and <= self.capacity_count, ignored Count):\n        ignored.release\n        self.initialized :: replace(self.initialized, next)\n        return\nrun:\n    region: Region(4, 1)\n    region.restore(region.initialized_count as Count, region.pop)\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "wrappers_capture_earlier_arguments_before_a_later_mutation",
        "Region has\n    restore(self mut, next Count and <= self.capacity_count, ignored Count):\n        ignored.release\n        self.initialized :: replace(self.initialized, next)\n        return\nrestore(region mut Region and region.initialized_count > 0):\n    region.restore(region.initialized_count as Count, region.pop)\n    return\nrun:\n    region: Region(4, 1)\n    restore(region)\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "a_mutated_scalar_input_cannot_supply_its_previous_constant",
        "clear_count(count mut is in Count):\n    count :: 0\n    return\nRegion has\n    copy_count(self mut and self.capacity_count = 255, count Count):\n        self.initialized :: replace(self.initialized, count)\n        return\nrun:\n    region: Region(255, 1)\n    count is in Count: 1\n    clear_count(count)\n    region.copy_count(count as Count)\n    value: region.last\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "rereading_an_unknown_field_does_not_restore_its_incoming_value",
        "choose(flag ref Count) Count:\n    when flag.as_num > 0 then\n        return 0\n    return 1\nRegion has\n    choose_count(self mut and self.capacity_count = 255, flag ref Count):\n        self.initialized :: replace(self.initialized, choose(flag))\n        return\nrewrite_count(region mut Region and region.capacity_count = 255, flag ref Count):\n    region.choose_count(flag)\n    count: region.initialized.as_num\n    region.initialized :: replace(region.initialized, count as Count)\n    return\nrun(flag Count):\n    region: Region(255, 1)\n    rewrite_count(region, flag)\n    value: region.last\n    value.release\n    region.release\n    flag.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "a_local_loop_counter_does_not_hide_unchanged_receiver_fields",
        "Region has\n    scan(self mut):\n        index is in Count: 0\n        while index < self.capacity_count do\n            index :: (next is in Count: index + 1)\n        return\nrun:\n    region: Region(4, 1)\n    region.scan\n    value: region.full_capacity\n    value.release\n    region.release\n    return\n",
        &[],
    ),
    (
        "a_loop_counter_cannot_publish_its_initial_value_after_the_loop",
        "Region has\n    scan(self mut and self.capacity_count = 255):\n        index is in Count: 0\n        while index < 2 do\n            index :: (next is in Count: index + 1)\n        self.initialized :: replace(self.initialized, index as Count)\n        return\nrun:\n    region: Region(255, 1)\n    region.scan\n    value: region.empty\n    value.release\n    region.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "rebinding_a_reference_preserves_both_owners_fields",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 1)\n    alias: mut left\n    alias :: mut right\n    first: left.full_capacity\n    first.release\n    second: right.full_capacity\n    second.release\n    left.release\n    right.release\n    return\n",
        &[],
    ),
    (
        "rebinding_a_reference_does_not_keep_the_old_targets_count",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 0)\n    alias: mut left\n    alias :: mut right\n    value: alias.last\n    value.release\n    left.release\n    right.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "rebinding_a_reference_preserves_an_earlier_scalar_read",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 0)\n    alias: mut left\n    count: alias.initialized_count\n    alias :: mut right\n    value: left.expect_count(count as Count)\n    value.release\n    left.release\n    right.release\n    return\n",
        &[],
    ),
    (
        "rebinding_a_reference_uses_the_new_targets_count",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 0)\n    alias: mut left\n    alias :: mut right\n    value: alias.empty\n    value.release\n    left.release\n    right.release\n    return\n",
        &[],
    ),
    (
        "a_saved_comparison_cannot_prove_a_rebound_references_count",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 0)\n    alias: mut left\n    ready: alias.initialized_count > 0\n    alias :: mut right\n    value: when ready then alias.last else (0 as Count)\n    value.release\n    left.release\n    right.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "a_loop_retargeting_an_alias_preserves_both_owners_capacity",
        "run:\n    left: Region(4, 1)\n    right: Region(4, 1)\n    alias: mut left\n    for index in 0 to 2\n        value: right.full_capacity\n        value.release\n        alias :: mut right\n        alias.clear\n    first: left.full_capacity\n    first.release\n    second: right.full_capacity\n    second.release\n    left.release\n    right.release\n    return\n",
        &[],
    ),
    (
        "a_loop_cannot_delay_an_alias_write_beyond_the_checked_iterations",
        "run:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    third: Region(4, 1)\n    writer: mut first\n    next: mut second\n    for index in 0 to 4\n        value: third.last\n        value.release\n        writer.clear\n        writer :: next\n        next :: mut third\n    first.release\n    second.release\n    third.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "cyclic_loop_alias_assignments_preserve_unchanged_fields",
        "run:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    writer: mut first\n    next: mut second\n    for index in 0 to 4\n        value: first.full_capacity\n        value.release\n        writer.clear\n        writer :: next\n        next :: writer\n    value: second.full_capacity\n    value.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "unresolved_loop_alias_sources_do_not_preserve_local_receiver_facts",
        "forward(region mut Region) mut Region: return region\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    third: Region(4, 1)\n    writer: mut first\n    next: mut second\n    for index in 0 to 4\n        value: third.last\n        value.release\n        writer.clear\n        writer :: next\n        next :: forward(third)\n    first.release\n    second.release\n    third.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "unresolved_loop_alias_sources_allow_a_fresh_receiver_guard",
        "forward(region mut Region) mut Region: return region\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    third: Region(4, 1)\n    writer: mut first\n    next: mut second\n    while third.initialized_count > 0 do\n        value: third.last\n        value.release\n        writer.clear\n        writer :: next\n        next :: forward(third)\n    first.release\n    second.release\n    third.release\n    return\n",
        &[],
    ),
    (
        "unresolved_loop_alias_sources_do_not_restore_facts_after_the_loop",
        "forward(region mut Region) mut Region: return region\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    writer: mut first\n    for index in 0 to 4\n        writer.clear\n        writer :: forward(second)\n    value: first.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "loop_alias_targets_include_the_incoming_reference",
        "run:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    writer: mut first\n    for index in 0 to 4\n        writer.clear\n        writer :: mut second\n    value: first.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "cyclic_loop_aliases_do_not_mix_different_owners_capacity",
        "run:\n    first: Region(4, 1)\n    second: Region(5, 1)\n    writer: mut first\n    next: mut second\n    for index in 0 to 4\n        writer.clear\n        writer :: next\n        next :: writer\n    value: second.full_capacity\n    value.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "a_stored_reference_write_invalidates_the_new_receivers_count",
        "Holder has\n    target mut Region\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    holder.target :: mut second\n    holder.target.clear\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "a_stored_reference_write_preserves_the_previous_receivers_count",
        "Holder has\n    target mut Region\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    holder.target :: mut second\n    holder.target.clear\n    value: first.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "a_loop_cannot_delay_a_stored_reference_write_beyond_checked_iterations",
        "Holder has\n    target mut Region\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    third: Region(4, 1)\n    writer: Holder(target: first)\n    next: Holder(target: second)\n    for index in 0 to 4\n        value: third.last\n        value.release\n        writer.target.clear\n        writer.target :: next.target\n        next.target :: mut third\n    writer.release\n    next.release\n    first.release\n    second.release\n    third.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "replacing_a_sequence_reference_invalidates_the_new_receivers_count",
        "run:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    pointers: [mut first]\n    pointers.[0] :: mut second\n    pointers.[0].clear\n    value: second.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "replacing_a_sequence_reference_preserves_the_previous_receivers_count",
        "run:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    pointers: [mut first]\n    pointers.[0] :: mut second\n    pointers.[0].clear\n    value: first.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "a_stored_reference_write_through_a_wrapper_invalidates_receiver_facts",
        "Holder has\n    target mut Region\n    release(self):\n        return\nclear_holder(holder mut Holder):\n    holder.target.clear\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    holder.target :: mut second\n    clear_holder(holder)\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "a_stored_reference_write_through_a_wrapper_preserves_the_old_receivers_facts",
        "Holder has\n    target mut Region\n    release(self):\n        return\nclear_holder(holder mut Holder):\n    holder.target.clear\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    holder.target :: mut second\n    clear_holder(holder)\n    value: first.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "nested_stored_reference_effects_reach_the_current_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nOuter has\n    holder mut Holder\n    release(self):\n        return\nclear_outer(outer mut Outer):\n    outer.holder.target.clear\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    holder.target :: mut second\n    outer: Outer(holder: holder)\n    clear_outer(outer)\n    value: second.last\n    value.release\n    outer.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "nested_stored_reference_effects_preserve_the_previous_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nOuter has\n    holder mut Holder\n    release(self):\n        return\nclear_outer(outer mut Outer):\n    outer.holder.target.clear\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    holder.target :: mut second\n    outer: Outer(holder: holder)\n    clear_outer(outer)\n    value: first.last\n    value.release\n    outer.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "writing_a_reference_through_an_alias_updates_the_container",
        "Holder has\n    target mut Region\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    alias: mut holder\n    alias.target :: mut second\n    holder.target.clear\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "writing_a_reference_through_an_alias_preserves_the_previous_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    alias: mut holder\n    alias.target :: mut second\n    holder.target.clear\n    value: first.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "writing_a_reference_through_the_container_updates_an_existing_alias",
        "Holder has\n    target mut Region\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    alias: mut holder\n    holder.target :: mut second\n    alias.target.clear\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[
            "type-parameter-refinement-unproven",
            "type-use-of-invalidated-reference",
        ],
    ),
    (
        "writing_a_reference_through_the_container_preserves_the_previous_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    alias: mut holder\n    holder.target :: mut second\n    alias.target.clear\n    value: first.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-use-of-invalidated-reference"],
    ),
    (
        "writing_a_sequence_reference_through_an_alias_updates_the_container",
        "run:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    pointers: [mut first]\n    alias: mut pointers\n    alias.[0] :: mut second\n    pointers.[0].clear\n    value: second.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "writing_a_sequence_reference_through_an_alias_preserves_the_previous_owner",
        "run:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    pointers: [mut first]\n    alias: mut pointers\n    alias.[0] :: mut second\n    pointers.[0].clear\n    value: first.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "writing_a_nested_reference_through_an_alias_updates_the_container",
        "Holder has\n    target mut Region\n    release(self):\n        return\nOuter has\n    holder mut Holder\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    outer: Outer(holder: holder)\n    nested: outer.holder\n    nested.target :: mut second\n    holder.target.clear\n    value: second.last\n    value.release\n    outer.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "writing_a_nested_reference_through_an_alias_preserves_the_previous_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nOuter has\n    holder mut Holder\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    outer: Outer(holder: holder)\n    nested: outer.holder\n    nested.target :: mut second\n    holder.target.clear\n    value: first.last\n    value.release\n    outer.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "replacing_a_stored_reference_inside_a_callee_invalidates_the_new_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget(holder mut Holder, next mut Region):\n    holder.target :: next\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    retarget(holder, second)\n    holder.target.clear\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "replacing_a_stored_reference_inside_a_callee_preserves_the_previous_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget(holder mut Holder, next mut Region):\n    holder.target :: next\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    retarget(holder, second)\n    holder.target.clear\n    value: first.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "a_wrapper_propagates_stored_reference_replacements",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget(holder mut Holder, next mut Region):\n    holder.target :: next\n    return\nforward(holder mut Holder, next mut Region):\n    retarget(holder, next)\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    forward(holder, second)\n    holder.target.clear\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "conditional_reference_replacement_retains_the_possible_first_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_if(holder mut Holder, next mut Region, flag ref Count):\n    when flag.as_num > 0 then\n        holder.target :: next\n        return\n    return\nrun(flag Count):\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    retarget_if(holder, second, flag)\n    holder.target.clear\n    value: first.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    flag.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "conditional_reference_replacement_retains_the_possible_second_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_if(holder mut Holder, next mut Region, flag ref Count):\n    when flag.as_num > 0 then\n        holder.target :: next\n        return\n    return\nrun(flag Count):\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    retarget_if(holder, second, flag)\n    holder.target.clear\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    flag.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "a_callee_cannot_store_a_reference_to_its_local_storage",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_local(holder mut Holder):\n    local: Region(4, 1)\n    holder.target :: mut local\n    local.release\n    return\n",
        &["type-reference-store-local-target"],
    ),
    (
        "recursive_stored_reference_writes_have_a_finite_summary",
        "Node has\n    next mut Node\n    release(self):\n        return\nrotate(node mut Node):\n    node.next :: node\n    rotate(node.next)\n    return\n",
        &[],
    ),
    (
        "retargeting_and_mutating_inside_a_callee_invalidates_the_new_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_and_clear(holder mut Holder, next mut Region):\n    holder.target :: next\n    holder.target.clear\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    retarget_and_clear(holder, second)\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "retargeting_and_mutating_inside_a_callee_preserves_the_previous_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_and_clear(holder mut Holder, next mut Region):\n    holder.target :: next\n    holder.target.clear\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    retarget_and_clear(holder, second)\n    value: first.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "retargeting_and_mutating_inside_a_callee_proves_the_new_owner_is_empty",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_and_clear(holder mut Holder, next mut Region):\n    holder.target :: next\n    holder.target.clear\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    retarget_and_clear(holder, second)\n    value: second.empty\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "a_wrapper_propagates_retarget_then_mutate_to_the_new_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_and_clear(holder mut Holder, next mut Region):\n    holder.target :: next\n    holder.target.clear\n    return\nforward(holder mut Holder, next mut Region):\n    retarget_and_clear(holder, next)\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    forward(holder, second)\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-failed"],
    ),
    (
        "a_wrapper_proves_the_retargeted_owner_is_empty",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_and_clear(holder mut Holder, next mut Region):\n    holder.target :: next\n    holder.target.clear\n    return\nforward(holder mut Holder, next mut Region):\n    retarget_and_clear(holder, next)\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    forward(holder, second)\n    value: second.empty\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "a_wrapper_preserves_the_previous_owners_scalar_facts",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_and_clear(holder mut Holder, next mut Region):\n    holder.target :: next\n    holder.target.clear\n    return\nforward(holder mut Holder, next mut Region):\n    retarget_and_clear(holder, next)\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    forward(holder, second)\n    value: first.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "conditional_retarget_then_mutate_invalidates_first",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_and_clear_if(holder mut Holder, next mut Region, flag ref Count):\n    when flag.as_num > 0 then\n        holder.target :: next\n        return\n    holder.target.clear\n    return\nrun(flag Count):\n    first: Region(4, 1)\n    second: Region(4, 1)\n    third: Region(4, 1)\n    holder: Holder(target: first)\n    retarget_and_clear_if(holder, second, flag)\n    value: first.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    third.release\n    flag.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "conditional_retarget_then_mutate_invalidates_second",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_and_clear_if(holder mut Holder, next mut Region, flag ref Count):\n    when flag.as_num > 0 then\n        holder.target :: next\n        return\n    holder.target.clear\n    return\nrun(flag Count):\n    first: Region(4, 1)\n    second: Region(4, 1)\n    third: Region(4, 1)\n    holder: Holder(target: first)\n    retarget_and_clear_if(holder, second, flag)\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    third.release\n    flag.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "conditional_retarget_then_mutate_preserves_an_unrelated_owner",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_and_clear_if(holder mut Holder, next mut Region, flag ref Count):\n    when flag.as_num > 0 then\n        holder.target :: next\n        return\n    holder.target.clear\n    return\nrun(flag Count):\n    first: Region(4, 1)\n    second: Region(4, 1)\n    third: Region(4, 1)\n    holder: Holder(target: first)\n    retarget_and_clear_if(holder, second, flag)\n    value: third.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    third.release\n    flag.release\n    return\n",
        &[],
    ),
    (
        "callee_fixed_array_reference_write_preserves_previous_owner",
        "retarget_array(items mut [mut Region; 1], next mut Region):\n    items.[0] :: next\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    items [mut Region; 1]: [mut first]\n    retarget_array(items, second)\n    items.[0].clear\n    value: first.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "callee_fixed_array_reference_write_invalidates_new_owner",
        "retarget_array(items mut [mut Region; 1], next mut Region):\n    items.[0] :: next\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    items [mut Region; 1]: [mut first]\n    retarget_array(items, second)\n    items.[0].clear\n    value: second.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "scalar_array_writes_preserve_unrelated_reference_origins",
        "write_count(items mut [Count; 1]):\n    items.[0] :: replace(items.[0], 0)\n    return\nHolder has\n    target mut Region\n    release(self):\n        return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    holder: Holder(target: first)\n    counts [Count; 1]: [0]\n    write_count(counts)\n    holder.target.clear\n    value: second.last\n    value.release\n    holder.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "wrapper_fixed_array_reference_write_preserves_previous_owner",
        "retarget_array(items mut [mut Region; 1], next mut Region):\n    items.[0] :: next\n    return\nforward_array(items mut [mut Region; 1], next mut Region):\n    retarget_array(items, next)\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    items [mut Region; 1]: [mut first]\n    forward_array(items, second)\n    items.[0].clear\n    value: first.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "fixed_array_element_mutation_invalidates_its_owner",
        "clear_array(items mut [mut Region; 1]):\n    items.[0].clear\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    items [mut Region; 1]: [mut first]\n    clear_array(items)\n    value: first.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "fixed_array_element_mutation_preserves_other_owners",
        "clear_array(items mut [mut Region; 1]):\n    items.[0].clear\n    return\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    items [mut Region; 1]: [mut first]\n    clear_array(items)\n    value: second.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "a_dynamic_array_write_cannot_store_a_local_reference",
        "retarget_local_array(items mut [mut Region; 2], index is in 0 to 1):\n    local: Region(4, 1)\n    items.[index] :: mut local\n    local.release\n    return\n",
        &["type-reference-store-local-target"],
    ),
    (
        "a_dynamic_array_write_through_a_wrapper_cannot_store_a_local_reference",
        "retarget_unknown(items mut [mut Region; 2], index is in 0 to 1, next mut Region):\n    items.[index] :: next\n    return\nforward_unknown(items mut [mut Region; 2], index is in 0 to 1, next mut Region):\n    retarget_unknown(items, index, next)\n    return\nstore_local(items mut [mut Region; 2], index is in 0 to 1):\n    local: Region(4, 1)\n    forward_unknown(items, index, local)\n    local.release\n    return\n",
        &["type-reference-store-local-target"],
    ),
    (
        "a_dynamic_array_write_can_store_a_local_reference_in_local_storage",
        "retarget_unknown(items mut [mut Region; 2], index is in 0 to 1, next mut Region):\n    items.[index] :: next\n    return\nforward_unknown(items mut [mut Region; 2], index is in 0 to 1, next mut Region):\n    retarget_unknown(items, index, next)\n    return\nrun(index is in 0 to 1):\n    first: Region(4, 1)\n    second: Region(4, 1)\n    items [mut Region; 2]: [mut first, mut first]\n    forward_unknown(items, index, second)\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "mutation_after_dynamic_array_retarget_invalidates_first",
        "retarget_then_clear(items mut [mut Region; 2], index is in 0 to 1, next mut Region):\n    items.[index] :: next\n    items.[0].clear\n    return\nrun(index is in 0 to 1):\n    first: Region(4, 1)\n    second: Region(4, 1)\n    other: Region(4, 1)\n    items [mut Region; 2]: [mut first, mut other]\n    retarget_then_clear(items, index, second)\n    value: first.last\n    value.release\n    first.release\n    second.release\n    other.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "mutation_after_dynamic_array_retarget_invalidates_second",
        "retarget_then_clear(items mut [mut Region; 2], index is in 0 to 1, next mut Region):\n    items.[index] :: next\n    items.[0].clear\n    return\nrun(index is in 0 to 1):\n    first: Region(4, 1)\n    second: Region(4, 1)\n    other: Region(4, 1)\n    items [mut Region; 2]: [mut first, mut other]\n    retarget_then_clear(items, index, second)\n    value: second.last\n    value.release\n    first.release\n    second.release\n    other.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "a_returned_fixed_array_cannot_contain_a_local_reference",
        "escape_array [mut Region; 1]:\n    local: Region(4, 1)\n    return [mut local]\n",
        &["type-reference-return-local-target"],
    ),
    (
        "a_returned_fixed_array_can_contain_caller_references",
        "forward_array(first mut Region, second mut Region) [mut Region; 2]: return [first, second]\n",
        &[],
    ),
    (
        "a_stored_fixed_array_cannot_contain_a_local_reference",
        "ArrayHolder has\n    targets [mut Region; 1]\n    release(self):\n        return\nstore_local_array(holder mut ArrayHolder):\n    local: Region(4, 1)\n    holder.targets :: [mut local]\n    local.release\n    return\n",
        &["type-reference-store-local-target"],
    ),
    (
        "an_empty_reference_array_has_no_escaping_elements",
        "empty_reference_array [mut Region; 0]: []\n",
        &[],
    ),
    (
        "a_returned_nested_fixed_array_cannot_contain_a_local_reference",
        "escape_nested_array [[mut Region; 1]; 1]:\n    local: Region(4, 1)\n    return [[mut local]]\n",
        &["type-reference-return-local-target"],
    ),
    (
        "a_reference_containing_array_parameter_can_be_forwarded",
        "forward_existing_array(items [mut Region; 1]) [mut Region; 1]: return items\n",
        &[],
    ),
    (
        "an_owned_array_parameter_cannot_return_a_new_local_reference",
        "replace_owned_array(items [mut Region; 1]) [mut Region; 1]:\n    working: items\n    local: Region(4, 1)\n    working.[0] :: mut local\n    return working\n",
        &["type-reference-return-local-target"],
    ),
    (
        "forwarded_array_mutation_invalidates_original_owner",
        "forward_existing_array(items [mut Region; 1]) [mut Region; 1]: return items\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    items [mut Region; 1]: [mut first]\n    returned: forward_existing_array(items)\n    returned.[0].clear\n    value: first.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &["type-parameter-refinement-unproven"],
    ),
    (
        "forwarded_array_mutation_preserves_unrelated_owner",
        "forward_existing_array(items [mut Region; 1]) [mut Region; 1]: return items\nrun:\n    first: Region(4, 1)\n    second: Region(4, 1)\n    items [mut Region; 1]: [mut first]\n    returned: forward_existing_array(items)\n    returned.[0].clear\n    value: second.last\n    value.release\n    first.release\n    second.release\n    return\n",
        &[],
    ),
    (
        "a_reference_containing_array_parameter_can_be_forwarded_through_a_wrapper",
        "forward_existing_array(items [mut Region; 1]) [mut Region; 1]: return items\nwrapper(items [mut Region; 1]) [mut Region; 1]: return forward_existing_array(items)\n",
        &[],
    ),
    (
        "nested_reference_arrays_can_be_forwarded",
        "forward_nested(items [[mut Region; 1]; 1]) [[mut Region; 1]; 1]: return items\n",
        &[],
    ),
    (
        "mutating_an_owned_arrays_referent_preserves_its_return_origin",
        "clear_owned(items [mut Region; 1]) [mut Region; 1]:\n    item: items.[0]\n    item.clear\n    return items\n",
        &[],
    ),
    (
        "retargeting_an_owned_arrays_referent_preserves_its_return_origin",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_owned(items [mut Holder; 1], next mut Region) [mut Holder; 1]:\n    item: items.[0]\n    item.target :: next\n    return items\n",
        &[],
    ),
    (
        "an_owned_arrays_referent_cannot_store_a_local_reference",
        "Holder has\n    target mut Region\n    release(self):\n        return\nstore_local_pointee(items [mut Holder; 1]) [mut Holder; 1]:\n    item: items.[0]\n    local: Region(4, 1)\n    item.target :: mut local\n    local.release\n    return items\n",
        &["type-reference-store-local-target"],
    ),
    (
        "owned_array_pointee_writes_preserve_wrapper_return_origins",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_owned(items [mut Holder; 1], next mut Region) [mut Holder; 1]:\n    item: items.[0]\n    item.target :: next\n    return items\nforward(items [mut Holder; 1], next mut Region) [mut Holder; 1]: return retarget_owned(items, next)\n",
        &[],
    ),
    (
        "owned_array_pointee_writes_through_a_wrapper_cannot_store_local_references",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_owned(items [mut Holder; 1], next mut Region) [mut Holder; 1]:\n    item: items.[0]\n    item.target :: next\n    return items\nstore_local(items [mut Holder; 1]) [mut Holder; 1]:\n    local: Region(4, 1)\n    result: retarget_owned(items, local)\n    local.release\n    return result\n",
        &["type-reference-store-local-target"],
    ),
    (
        "borrowed_array_pointee_writes_through_a_wrapper_cannot_store_local_references",
        "Holder has\n    target mut Region\n    release(self):\n        return\nretarget_borrowed(items mut [mut Holder; 1], next mut Region):\n    item: items.[0]\n    item.target :: next\n    return\nstore_local(items mut [mut Holder; 1]):\n    local: Region(4, 1)\n    retarget_borrowed(items, local)\n    local.release\n    return\n",
        &["type-reference-store-local-target"],
    ),
    (
        "preservation_proofs_on_all_explicit_branch_returns_reach_the_caller",
        "Choice is Left or Right\npreserve(region mut Region, choice Choice):\n    before: region.initialized_count\n    when choice is\n        Left then return thus region.initialized_count = before\n        Right then return thus region.initialized_count = before\nrun:\n    region: Region(4, 1)\n    preserve(region, Left)\n    last: region.last\n    last.release\n    region.release\n    return\n",
        &[],
    ),
];

#[test]
fn mutation_summary_cases() {
    for &(name, body, expected) in CASES {
        expect_flaws_with_named(TYPES, body, expected, name);
    }
}

#[test]
fn nested_mutation_summaries_track_each_decrement() {
    let wrapper = "Wrapper has\n    region Region\n    pop(self mut and self.region.initialized_count > 0) Count: return self.region.pop\n    empty(self ref and self.region.initialized_count = 0) Count: return 0\n    release(self):\n        self.region.release\n        return\n";
    expect_flaws_with(
        TYPES,
        &format!(
            "{wrapper}run:\n    wrapper: Wrapper(region: Region(4, 2))\n    first: wrapper.pop\n    first.release\n    second: wrapper.pop\n    second.release\n    value: wrapper.empty\n    value.release\n    wrapper.release\n    return\n"
        ),
        &[],
    );
    expect_flaws_with(
        TYPES,
        &format!(
            "{wrapper}run:\n    wrapper: Wrapper(region: Region(4, 1))\n    first: wrapper.pop\n    first.release\n    second: wrapper.pop\n    second.release\n    wrapper.release\n    return\n"
        ),
        &["type-parameter-refinement-failed"],
    );
}

const SWAP_MODEL: &str = "Pair has\n    left Count\n    right Count\n    swap(self mut):\n        saved: self.left.as_num\n        self.left :: replace(self.left, self.right.as_num as Count)\n        self.right :: replace(self.right, saved as Count)\n        return\n    swapped_left(self ref and self.left.as_num = 2) Count: return 0\n    swapped_right(self ref and self.right.as_num = 1) Count: return 0\n    old_left(self ref and self.left.as_num = 1) Count: return 0\n    release(self):\n        return\n";

#[test]
fn field_swap_summaries_use_both_original_field_values() {
    expect_flaws_with(
        TYPES,
        &format!(
            "{SWAP_MODEL}run:\n    pair: Pair(left: 1, right: 2)\n    pair.swap\n    left: pair.swapped_left\n    left.release\n    right: pair.swapped_right\n    right.release\n    pair.release\n    return\n"
        ),
        &[],
    );
}

#[test]
fn field_swap_summaries_do_not_restore_the_old_left_value() {
    expect_flaws_with(
        TYPES,
        &format!(
            "{SWAP_MODEL}run:\n    pair: Pair(left: 1, right: 2)\n    pair.swap\n    left: pair.old_left\n    left.release\n    pair.release\n    return\n"
        ),
        &["type-parameter-refinement-failed"],
    );
}

#[test]
fn mutation_guarantees_cross_module_boundaries_in_either_order() {
    let client = "use sample.model Count, Region\nrun:\n    region: Region(capacity: 4, initialized: 1)\n    taken: region.pop\n    taken.release\n    value: region.empty\n    value.release\n    region.release\n    return\n";
    for reversed in [false, true] {
        let mut files = [("model", TYPES), ("client", client)];
        if reversed {
            files.reverse();
        }
        let output = package("sample", &files);
        let flaws: Vec<_> = output.iter().flat_map(|file| file.all_flaws()).collect();
        assert!(flaws.is_empty(), "{flaws:?}");
    }
}

#[test]
fn editing_a_mutation_updates_cached_proofs_and_the_public_surface() {
    use ast::ty::PackageInstanceKey;
    use typecheck::transform::{
        BodyReuse, PackageCheckSession, PackageTransformOptions,
        transform_package_with_shared_context_and_session,
    };

    let mut session =
        PackageCheckSession::new(1.try_into().unwrap(), BodyReuse::ExactInputs).unwrap();
    let mut fingerprints = Vec::new();
    let package = PackageInstanceKey::workspace("sample", "1");
    let client = "use sample.model Count, Region\nrun:\n    region: Region(capacity: 4, initialized: 2)\n    taken: region.pop\n    taken.release\n    value: region.empty\n    value.release\n    region.release\n    return\n";
    for clear in [false, false, true, true, false] {
        let model = if clear {
            TYPES.replace("index as Count)", "0)")
        } else {
            TYPES.to_owned()
        };
        let files = [("model", model.as_str()), ("client", client)]
            .into_iter()
            .map(|(module, source)| {
                let mut parsed = source.parse_source_full();
                assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
                parsed.ast.semantic_origin = Some(ast::prelude::FileSemanticOrigin {
                    package: package.clone(),
                    module: vec![internment::Intern::from_ref(module)],
                });
                parsed.ast
            })
            .collect();
        let output = transform_package_with_shared_context_and_session(
            files,
            PackageTransformOptions::FULL,
            package.clone(),
            &mut session,
        );
        let codes: Vec<_> = output.typed_asts.iter().flat_map(flaw_codes).collect();
        let expected = if clear {
            vec![]
        } else {
            vec!["type-parameter-refinement-failed"]
        };
        assert_eq!(codes, expected);
        fingerprints.push(output.public_interface.semantic_surface_fingerprint);
    }
    assert_eq!(fingerprints[0], fingerprints[1]);
    assert_ne!(fingerprints[1], fingerprints[2]);
    assert_eq!(fingerprints[2], fingerprints[3]);
    assert_eq!(fingerprints[0], fingerprints[4]);
}

#[test]
fn conditional_maximum_keeps_the_bound_proven_on_every_arm() {
    expect_flaws_with(
        TYPES,
        "at_least(value Count, bound Count and bound <= value):\n    value.release\n    bound.release\n    return\ncheck(left ref Count, right ref Count):\n    result: when left.as_num < right.as_num then right.as_num else left.as_num\n    at_least(result as Count, left.as_num as Count)\n    return\n",
        &[],
    );
    expect_flaws_with(
        TYPES,
        "at_most(value Count, bound Count and value <= bound):\n    value.release\n    bound.release\n    return\ncheck(left ref Count, right ref Count):\n    result: when left.as_num < right.as_num then right.as_num else left.as_num\n    at_most(result as Count, left.as_num as Count)\n    return\n",
        &["type-parameter-refinement-unproven"],
    );
}

#[test]
fn a_large_array_reference_write_tracks_only_the_written_slot() {
    let typed = check(&format!(
        "{TYPES}retarget_large(items mut [mut Region; 1000000], next mut Region):\n    items.[999999] :: next\n    return\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let summary = &typed.defs
        [&typecheck::DefId::named(internment::Intern::from_ref("retarget_large"))]
        .references;
    assert!(!summary.unknown_writes);
    assert_eq!(summary.writes.len(), 1);
    let (target, value) = summary.writes.iter().next().unwrap();
    assert_eq!(target.base.parameter, 0);
    assert!(target.base.fields.is_empty());
    assert_eq!(target.field, 999999);
    assert_eq!(value.fields.len(), 2);
    assert!(
        value
            .sources
            .contains(&typecheck::typed::ReferenceSource::Argument(
                typecheck::typed::ArgumentReference {
                    parameter: 1,
                    fields: Vec::new()
                }
            ))
    );
}

#[test]
fn a_dynamic_array_reference_write_reports_unknown_slots() {
    let typed = check(&format!(
        "{TYPES}retarget_unknown(items mut [mut Region; 2], index is in 0 to 1, next mut Region):\n    items.[index] :: next\n    return\n"
    ));
    assert!(typed.all_flaws().is_empty(), "{:?}", typed.all_flaws());
    let summary = &typed.defs
        [&typecheck::DefId::named(internment::Intern::from_ref("retarget_unknown"))]
        .references;
    assert!(summary.unknown_writes);
}

#[test]
fn reference_write_payload_changes_invalidate_cached_callers_and_public_contracts() {
    let client = "use sample.model Count, Region, Holder, retarget\nrun:\n    left: Region(capacity: 4, initialized: 1)\n    right: Region(capacity: 4, initialized: 1)\n    holder: Holder(target: left)\n    retarget(holder, left, right)\n    holder.target.clear\n    value: right.last\n    value.release\n    holder.release\n    left.release\n    right.release\n    return\n";
    check_reference_payload_cache(
        client,
        |right| {
            let selected = if right { "right" } else { "left" };
            let model = format!(
                "{TYPES}Holder has\n    target mut Region\n    release(self):\n        return\nretarget(holder mut Holder, left mut Region, right mut Region):\n    first: left.initialized_count as Count\n    first.release\n    second: right.initialized_count as Count\n    second.release\n    holder.target :: {selected}\n    return\n"
            );
            model
        },
        |right| {
            if right {
                vec!["type-parameter-refinement-unproven"]
            } else {
                vec![]
            }
        },
    );
}

#[test]
fn element_write_payload_changes_invalidate_cached_callers_and_public_contracts() {
    let client = "use sample.model Count, Region, Holder, retarget_owned\nstore_local(items [mut Holder; 1], next mut Region) [mut Holder; 1]:\n    local: Region(capacity: 4, initialized: 1)\n    result: retarget_owned(items, local, next)\n    local.release\n    return result\n";
    check_reference_payload_cache(
        client,
        |right| {
            let selected = if right { "right" } else { "left" };
            let model = format!(
                "{TYPES}Holder has\n    target mut Region\n    release(self):\n        return\nretarget_owned(items [mut Holder; 1], left mut Region, right mut Region) [mut Holder; 1]:\n    first: left.initialized_count as Count\n    first.release\n    second: right.initialized_count as Count\n    second.release\n    item: items.[0]\n    item.target :: {selected}\n    return items\n"
            );
            model
        },
        |right| {
            if right {
                vec![]
            } else {
                vec!["type-reference-store-local-target"]
            }
        },
    );
}

fn check_reference_payload_cache(
    client: &str,
    model_for: impl Fn(bool) -> String,
    expected_for: impl Fn(bool) -> Vec<&'static str>,
) {
    use ast::ty::PackageInstanceKey;
    use typecheck::transform::{
        BodyReuse, PackageCheckSession, PackageTransformOptions,
        transform_package_with_shared_context_and_session,
    };

    let package = PackageInstanceKey::workspace("sample", "1");
    let mut session =
        PackageCheckSession::new(1.try_into().unwrap(), BodyReuse::ExactInputs).unwrap();
    let mut fingerprints = Vec::new();
    for right in [false, false, true, true, false] {
        let model = model_for(right);
        let files = [("model", model.as_str()), ("client", client)]
            .into_iter()
            .map(|(module, source)| {
                let mut parsed = source.parse_source_full();
                assert!(parsed.symptoms.is_empty(), "{:?}", parsed.symptoms);
                parsed.ast.semantic_origin = Some(ast::prelude::FileSemanticOrigin {
                    package: package.clone(),
                    module: vec![internment::Intern::from_ref(module)],
                });
                parsed.ast
            })
            .collect();
        let output = transform_package_with_shared_context_and_session(
            files,
            PackageTransformOptions::FULL,
            package.clone(),
            &mut session,
        );
        let codes: Vec<_> = output.typed_asts.iter().flat_map(flaw_codes).collect();
        let expected = expected_for(right);
        assert_eq!(
            codes,
            expected,
            "{:#?}",
            output
                .typed_asts
                .iter()
                .flat_map(|file| {
                    file.exprs
                        .kind
                        .iter()
                        .enumerate()
                        .filter_map(|(index, kind)| {
                            let flaws = &file.exprs.facts.semantic_facts.flaws[index];
                            (!flaws.is_empty()).then(|| (kind, &file.exprs.ty[index], flaws))
                        })
                })
                .collect::<Vec<_>>()
        );
        fingerprints.push(output.public_interface.semantic_surface_fingerprint);
    }
    assert_eq!(fingerprints[0], fingerprints[1]);
    assert_ne!(fingerprints[1], fingerprints[2]);
    assert_eq!(fingerprints[2], fingerprints[3]);
    assert_eq!(fingerprints[0], fingerprints[4]);
}

#[test]
fn loop_helpers_preserve_only_fields_they_do_not_write() {
    let helper = "Region has\n    drain(self mut):\n        while self.initialized_count > 0 do\n            value: self.pop\n            value.release\n        return\n    resize(self mut):\n        for index in 0 to 1\n            self.capacity :: replace(self.capacity, 255)\n        return\n";
    expect_flaws_with(
        TYPES,
        &format!(
            "{helper}run:\n    region: Region(4, 2)\n    region.drain\n    value: region.full_capacity\n    value.release\n    region.release\n    return\n"
        ),
        &[],
    );
    expect_flaws_with(
        TYPES,
        &format!(
            "{helper}run:\n    region: Region(4, 2)\n    region.resize\n    value: region.full_capacity\n    value.release\n    region.release\n    return\n"
        ),
        &["type-parameter-refinement-unproven"],
    );
}
