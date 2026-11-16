//! Soft voice budget: voices past the full-mix budget are demoted (attenuated
//! per rank, panned centred) instead of culled. See
//! docs/progress/2026-11-15-audio-soft-voice-budget.md.
mod common;
use bri_audio::*;
use common::*;

fn offline(cfg: impl FnOnce(&mut RuntimeConfig)) -> AudioRuntime {
    let mut c = RuntimeConfig::default();
    c.engine.sample_rate = OUT_RATE;
    c.engine.gain_curve = GainCurve::Linear;
    cfg(&mut c);
    AudioRuntime::new(bank(), c, OutputKind::Offline).unwrap()
}

/// Eight world booms at increasing distance over a full-mix budget of two:
/// all eight start and mix (none culled), six are demoted, and the demoted
/// run carries measurably less energy than the same scene undemoted.
#[test]
fn dense_scene_demotes_instead_of_culling() {
    // Keep every voice far under the master limiter so the assertion below
    // measures the mix network's energy, not the limiter's output ceiling.
    let scene = |c: &mut RuntimeConfig, factor: f32| {
        c.engine.voices.max_real_voices = 2;
        c.engine.voices.max_mixed_voices = 8;
        c.engine.voices.demotion_gain_per_rank = factor;
    };

    let mut demoted = offline(|c| scene(c, 0.5));
    let mut undemoted = offline(|c| scene(c, 1.0));
    for rt in [&mut demoted, &mut undemoted] {
        rt.set_volume(VolumeControl::Master, 0.05).unwrap();
        for d in [1.0f32, 5.0, 10.0, 15.0, 20.0, 25.0, 30.0, 35.0] {
            rt.play("BoomSound", Placement::World([0.0, 0.0, -d]))
                .unwrap();
        }
    }
    let a = demoted.render_offline(OUT_RATE as usize / 10); // 0.1 s
    let b = undemoted.render_offline(OUT_RATE as usize / 10);

    let sa = demoted.stats();
    // Nothing rejected, culled or virtualized: the hard cap is 8 and eight
    // voices started. (Vanilla's 16 is the full-mix budget, not the cap.)
    assert_eq!((sa.rejected, sa.culled, sa.virtualized), (0, 0, 0));
    assert_eq!((sa.real_voices, sa.demoted_voices), (8, 6));
    // The two loudest keep full gain, so the cut is bounded by them; the six
    // over budget fade per rank and the mix energy drops by roughly sqrt of
    // the lost per-voice energy (~0.8x here).
    assert!(
        rms(&a) < rms(&b) * 0.9,
        "demotion should cut dense-scene energy: {} vs {}",
        rms(&a),
        rms(&b)
    );
    // Demotion never clips or produces non-finite output.
    assert_eq!((sa.clipped_samples, sa.non_finite_samples), (0, 0));

    // Control: the undemoted run has no demoted voices at all.
    let sb = undemoted.stats();
    assert_eq!((sb.real_voices, sb.demoted_voices), (8, 0));
}

/// Defaults unchanged = vanilla: the full-mix budget is also the hard cap, so
/// a twentieth equal-loudness boom is rejected under voice pressure, exactly
/// as before this change, and no voice is ever demoted.
#[test]
fn default_policy_still_hard_culls_at_sixteen() {
    let mut rt = offline(|_| {});
    for _ in 0..20 {
        rt.play("BoomSound", Placement::World([0.0, 0.0, -1.0]))
            .unwrap();
    }
    rt.render_offline(OUT_RATE as usize / 100);
    let s = rt.stats();
    assert_eq!(s.real_voices, 16);
    assert_eq!(s.demoted_voices, 0);
    assert_eq!(s.rejected, 4); // equal scores: nothing quieter to evict
    assert_eq!(s.culled, 0);
}

/// A demoted voice keeps its distance attenuation but loses its direction:
/// a hard-right loop pans centred once the demotion applies, while the same
/// loop at full mix stays hard-right.
#[test]
fn demoted_voice_pans_centred() {
    // LoopSound: spatial (5, 30); at [20, 0, 0] the attenuation is 0.4 and
    // the listener (facing -Z) has +X as right, so full mix is hard-right.
    let pan_case = |budget: usize| -> (f32, f32) {
        let mut rt = offline(|c| {
            c.engine.voices.max_real_voices = budget;
            c.engine.voices.max_mixed_voices = 8;
            c.engine.voices.demotion_gain_per_rank = 0.9;
        });
        rt.play("LoopSound", Placement::World([20.0, 0.0, 0.0]))
            .unwrap();
        let out = rt.render_offline(OUT_RATE as usize * 8 / 10); // 0.8 s
        // Skip the first blocks (the voice starts at full-mix pan and the
        // ramp settles): the tail reflects the steady demoted state.
        let tail = &out[out.len() - OUT_RATE as usize / 2 * 2..];
        rms_lr(tail)
    };
    let (full_l, full_r) = pan_case(1); // within budget: full mix
    let (dem_l, dem_r) = pan_case(0); // over budget: demoted
    assert!(
        full_l < full_r * 0.02,
        "full mix must pan right: {full_l} {full_r}"
    );
    assert!(
        (dem_l - dem_r).abs() < dem_r.max(1e-6) * 0.02,
        "demoted voice must pan centred: {dem_l} {dem_r}"
    );
    assert!(dem_r > 0.0, "demoted voice must still be audible: {dem_r}");
}
