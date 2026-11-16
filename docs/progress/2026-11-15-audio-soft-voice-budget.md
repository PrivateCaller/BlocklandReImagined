# Soft voice budget: demote dense-scene voices instead of culling

Maxwell asked whether the vanilla 16-voice culling could keep dense scenes
legible instead of dropping sounds outright, specifically by de-spatializing
or ducking the excess rather than culling it. This change adds that as an
opt-in mixing policy in `bri-audio`; shipped defaults reproduce vanilla
behaviour exactly, so no existing behaviour changes until a mix opts in.

`VoicePolicy` gains two fields. `max_real_voices` keeps its meaning but is
now the *full-mix budget*: the voices that keep their authored gain and
spatial pan. `max_mixed_voices` is the new hard cap on simultaneously mixed
voices, and `demotion_gain_per_rank` is the per-rank multiplier for voices
over the budget: the `r`-th voice past the budget mixes at `factor^r`
(0.708 is -3 dB). `Engine::new` sanitizes both (`max_mixed_voices >=
max_real_voices`, finite `0..=1` factor) and sizes the voice pool from the
hard cap. The defaults `max_mixed_voices = 16` and `max_real_voices = 16`
make the budget the cap, so demotion never fires and eviction, culling and
virtualization behave exactly as before — the regression test
`default_policy_still_hard_culls_at_sixteen` locks that in.

`Engine::refresh_demotion` runs once per mix block before gain-target
updates: it ranks the live, unsuspended real voices by last block's score
(stable on score ties) and assigns each voice over the budget its
`factor^rank-over` demotion. Ranking deliberately uses undemoted scores, so
the vanilla eviction ranking, hysteresis and virtual/uncull behaviour above
the hard cap are untouched; demotion only changes what is *heard*, never who
owns a slot. `gains` then multiplies the post-curve amplitude by the
demotion factor and, for demoted voices, replaces the directional pan with a
constant-power centred pan while keeping distance attenuation. Rationale:
the excess voices in a dense scene are the ones a listener could not
localize anyway (quiet, distant, masked); folding them into an attenuated
centred bed preserves event presence and density without steering the stereo
field away from the sounds inside the full-mix budget. `AudioStats`/
`SharedStats` publish `demoted_voices` for the probes overlay, and
`start`/`revive_virtual` now test the hard cap rather than the budget.

Suggested tuning for dense scenes is `max_real_voices = 16` (unchanged
legibility for the loud, close sounds), `max_mixed_voices = 32..48` and
`demotion_gain_per_rank ≈ 0.708`; OpenAL Soft's 256-voice headroom is not
needed because demotion reaches perceived continuity well below it — beyond
the audible floor of -20 dB per rank the exponential taper makes further
voices inaudible regardless. This is a mixing-policy change, not a device
change: cpal output, the ring-buffer command path and the client pass-through
(`app/probes.rs`) were verified to compile against the widened stats struct.

Verification (`cargo fmt`/`clippy -D warnings` clean on `bri-audio`,
`cargo check -p bri-client` clean): the full `cargo test -p bri-audio` suite
passes — 86 tests, all green. The new `tests/demotion.rs` adds three offline
render tests: eight booms over a two-voice budget with an eight-voice cap
start fully (zero rejected/culled/virtualized), six report demoted, and the
mix carries measurably less energy than the same scene undemoted; a single
hard-right loop pans hard-right inside the budget and measured-identical in
both channels once demoted; and the default policy still rejects a
twentieth equal-loudness boom under voice pressure with zero demoted voices.
One existing `sustain.rs` test had pinned `max_real_voices = 0` to force
virtualization; it now pins `max_mixed_voices = 0` too, since the hard-cap
knob moved. No playtest acceptance is claimed — the audible tuning of the
per-rank curve remains Maxwell's call.
