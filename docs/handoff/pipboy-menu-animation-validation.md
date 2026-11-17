# Pip-Boy first-person script animation validation

The device-menu clock and selected script-layer blends are implemented from
[the mapped behavior note](../spec-notes/pipboy.md#first-person-script-animations-while-the-device-is-open).
The owner's holstered/no-weapon invisible opening is the target. A log reporting
`raised` or menus on the model does not prove visible output.

| Boundary | Authored selector | Execution |
|---|---|---|
| Source compiler, inventory freeze, first-person sample/getters/setters, NPC/third-person controls, unrelated menus/loading, deferred seek, deactivation/resume, automatic completion and fresh runtime | `cargo test -p fnv-game --test first_person_menu_clock -- --nocapture` | Unexecuted |
| Real layered CPU pose, priority-1 script above ordinary Pip-Boy hold, world delta zero, holstered/drawn hands, menu fade-in/out and third-person control | `cargo test -p nvforever player_body::sequence_consumer_tests::device_menu_sequence_moves_and_blends_with_holstered_or_drawn_hands_and_a_held_world -- --exact --nocapture` from `engine` | Unexecuted |
| Existing source clock/transition controls | `cargo test -p fnv-game --test small_libs explicit_player_sequences_keep_view_clocks_targets_and_release_state -- --exact` and `transition_helpers_preserve_explicit_weight_and_offset_readbacks` | Unexecuted |
| Existing prepared-sample CPU pose control | `cargo test -p nvforever player_body::sequence_consumer_tests::prepared_public_sequence_reaches_real_pose_and_releases_to_the_underlying_clip -- --exact` from `engine` | Unexecuted |
| Existing blend/fade controls after sharing the fade step | `cargo test -p nvf-assets --lib anim_blend::tests` from `engine` | Unexecuted |

The CPU pose fixture supplies an authored decoded clip at the existing prepared
sample seam. It does not execute asynchronous KF loading, the complete renderer
schedule, a weapon model or the installed mod scripts. No game assets belong in
these fixtures.

The core clock fixture uses only interfaces present in the accepted baseline and
can run unchanged there. The new CPU pose fixture uses the candidate's explicit
menu blend-time seam; an original-source run must declare its adapter for that
field rather than count a baseline compilation error as the expected failure.

The real launcher boundary still needs baseline and candidate checks with the
same installed profile: Pip-Boy opening with no weapon, a holstered weapon and a
drawn weapon; close/cancel/reopen; combat/reload control; menu pause modes; and
save/load. The separately observed zero-submission readiness cap remains open.
Native pause parity and complete installed-mod compatibility are unqualified.
