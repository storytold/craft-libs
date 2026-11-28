#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::edit::*;
use crate::format::ErrorKind;
use crate::{ColorToken, ContrastIntent, IconPackId, Mode, ModePins, Rgba, ThemeDef, Variants, builtins};

fn builtin(id: &str) -> ThemeDef {
    builtins().iter().find(|t| t.id.as_str() == format!("craft.{id}")).unwrap().clone()
}

/// A theme with both modes and no pins of its own.
fn bare() -> ThemeDef {
    let mut t = builtin("graphite");
    t.pins = ModePins::default();
    t.high_contrast = ModePins::default();
    t.apps.clear();
    t
}

fn has(errors: &crate::format::FormatErrors, f: impl Fn(&ErrorKind) -> bool) -> bool {
    errors.0.iter().any(|e| f(&e.kind))
}

fn pin(token: ColorToken, color: Rgba) -> Edit {
    Edit::SetPin { slot: PinSlot::theme(), mode: Mode::Light, token, color }
}

#[test]
fn pin_then_revert_gives_saved() {
    let saved = bare();
    let draft = apply(&saved, &pin(ColorToken::TextMuted, Rgba::rgb(10, 20, 30))).unwrap();
    let changes = diff(&saved, &draft);
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].kind, ChangeKind::Added);
    assert_eq!(changes[0].revert(&draft).unwrap(), saved);
}

#[test]
fn pin_for_missing_mode_is_refused() {
    let mut saved = builtin("classic");
    assert_eq!(saved.variants, Variants::LightOnly);
    saved.pins = ModePins::default();
    let edit = Edit::SetPin { slot: PinSlot::theme(), mode: Mode::Dark, token: ColorToken::TextMuted, color: Rgba::rgb(1, 2, 3) };
    let errors = apply(&saved, &edit).unwrap_err();
    assert!(has(&errors, |k| matches!(k, ErrorKind::NoSuchMode)));
}

#[test]
fn translucent_surface_pin_is_refused() {
    let saved = bare();
    let errors = apply(&saved, &pin(ColorToken::SurfacePanel, Rgba::rgb(1, 2, 3).with_alpha(100))).unwrap_err();
    assert!(has(&errors, |k| matches!(k, ErrorKind::NotOpaque)));
    assert!(apply(&saved, &pin(ColorToken::SurfacePanel, Rgba::rgb(1, 2, 3))).is_ok());
}

#[test]
fn reference_cycle_is_refused() {
    let follow = |token, target| Edit::SetReference { slot: PinSlot::theme(), mode: Mode::Light, token, target };
    let t = apply(&bare(), &follow(ColorToken::AccentHover, ColorToken::AccentPressed)).unwrap();
    let errors = apply(&t, &follow(ColorToken::AccentPressed, ColorToken::AccentHover)).unwrap_err();
    assert!(has(&errors, |k| matches!(k, ErrorKind::ReferenceCycle { .. })));
}

#[test]
fn diff_lists_each_change_and_reverts_alone() {
    let follow = |token, target| Edit::SetReference { slot: PinSlot::theme(), mode: Mode::Light, token, target };
    let mut saved = bare();
    for edit in [pin(ColorToken::TextMuted, Rgba::rgb(9, 9, 9)), follow(ColorToken::AccentHover, ColorToken::AccentSolid)] {
        saved = apply(&saved, &edit).unwrap();
    }
    let mut draft = saved.clone();
    for edit in [
        Edit::SetSeed { seed: Seed::Accent, color: Rgba::rgb(200, 30, 60) },
        pin(ColorToken::SurfaceRaised, Rgba::rgb(240, 240, 240)),
        Edit::ClearPin { slot: PinSlot::theme(), mode: Mode::Light, token: ColorToken::TextMuted },
        follow(ColorToken::AccentHover, ColorToken::AccentPressed),
    ] {
        draft = apply(&draft, &edit).unwrap();
    }
    let changes = diff(&saved, &draft);
    let kinds: Vec<ChangeKind> = changes.iter().map(|c| c.kind).collect();
    assert_eq!(changes.len(), 4);
    assert_eq!(kinds.iter().filter(|&&k| k == ChangeKind::Added).count(), 1);
    assert_eq!(kinds.iter().filter(|&&k| k == ChangeKind::Removed).count(), 1);
    assert_eq!(kinds.iter().filter(|&&k| k == ChangeKind::Changed).count(), 2);

    for (i, change) in changes.iter().enumerate() {
        let reverted = change.revert(&draft).unwrap();
        let rest = diff(&saved, &reverted);
        let expected: Vec<&Key> = changes.iter().enumerate().filter(|&(j, _)| j != i).map(|(_, c)| &c.key).collect();
        let left: Vec<&Key> = rest.iter().map(|c| &c.key).collect();
        assert_eq!(left, expected, "reverting {:?}", change.key);
    }
}

#[test]
fn meta_and_mode_seed_changes_revert() {
    let saved = bare();
    let mut draft = saved.clone();
    for edit in [
        Edit::SetMeta(Meta::ContrastIntent(ContrastIntent::Low)),
        Edit::SetMeta(Meta::IconPack(Some(IconPackId::parse("craft.tabler").unwrap()))),
        Edit::SetModeSeed { mode: Mode::Dark, seed: Seed::Neutral, color: Some(Rgba::rgb(40, 40, 50)) },
    ] {
        draft = apply(&draft, &edit).unwrap();
    }
    let mut reverted = draft.clone();
    for change in diff(&saved, &draft) {
        reverted = change.revert(&reverted).unwrap();
    }
    assert_eq!(reverted, saved);
}
