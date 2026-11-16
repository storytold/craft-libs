use super::*;
use crate::ids::ModelId;

fn cap(s: &str) -> CapabilityId {
    CapabilityId::parse(s).unwrap()
}

fn model(id: &str, price: Option<u64>, released: Option<u64>) -> ModelInfo {
    let mut m = ModelInfo::new(ModelId::new(id).unwrap(), id);
    m.pricing = price.map(|p| Pricing { per_image_micros: Some(p), ..Pricing::default() });
    m.released = released;
    m
}

fn cands<'a>(models: &'a [(&'a str, ModelInfo, bool)]) -> Vec<Candidate<'a>> {
    models.iter().map(|(provider, model, free)| Candidate { provider, model, free: *free }).collect()
}

fn catalog(entries: &[(&str, &str, &str, &str)]) -> Catalog {
    Catalog {
        version: 1,
        entries: entries
            .iter()
            .map(|(capability, pick, provider, model)| CatalogEntry {
                capability: (*capability).into(),
                pick: (*pick).into(),
                provider: (*provider).into(),
                model: (*model).into(),
            })
            .collect(),
    }
}

fn find(picks: &[Picked], pick: Pick) -> Option<&Picked> {
    picks.iter().find(|p| p.pick == pick)
}

#[test]
fn the_built_in_catalog_loads_and_starts_empty() {
    let c = Catalog::builtin();
    assert_eq!(c.version, 1);
    assert!(c.entries.is_empty(), "no curated pick is claimed until it is checked");
}

#[test]
fn lowest_cost_and_newest_are_worked_out_from_what_is_reported() {
    let models = [
        ("or", model("a-pricey", Some(80_000), Some(300)), false),
        ("or", model("b-cheap", Some(20_000), Some(100)), false),
        ("or", model("c-new", Some(50_000), Some(900)), false),
        ("or", model("d-unpriced", None, None), false),
    ];
    let picks = choose(&cap("image.inpaint"), &cands(&models), &Catalog::default());
    let cheapest = find(&picks, Pick::LowestCost).unwrap();
    assert_eq!((cheapest.candidate, &cheapest.by), (1, &PickedBy::Price(20_000)));
    let newest = find(&picks, Pick::Newest).unwrap();
    assert_eq!((newest.candidate, &newest.by), (2, &PickedBy::Released(900)));
    assert!(find(&picks, Pick::Recommended).is_none(), "curated picks need the catalog");
}

#[test]
fn a_free_model_is_the_lowest_cost_and_unpriced_hosted_models_are_not_guessed_at() {
    let models = [
        ("or", model("hosted", Some(1), None), false),
        ("comfyui", model("local-sdxl", None, None), true),
        ("or", model("unpriced", None, None), false),
    ];
    let picks = choose(&cap("image.generate"), &cands(&models), &Catalog::default());
    let cheapest = find(&picks, Pick::LowestCost).unwrap();
    assert_eq!((cheapest.candidate, &cheapest.by), (1, &PickedBy::Free));

    let only_unpriced = [("or", model("x", None, None), false)];
    let picks = choose(&cap("image.generate"), &cands(&only_unpriced), &Catalog::default());
    assert!(picks.is_empty(), "no data, no pick: {picks:?}");
}

#[test]
fn text_models_compare_input_plus_output_per_million_tokens() {
    let text = |id: &str, i: u64, o: u64| {
        let mut m = ModelInfo::new(ModelId::new(id).unwrap(), id);
        m.pricing = Some(Pricing { input_micros_per_mtok: Some(i), output_micros_per_mtok: Some(o), per_image_micros: None });
        m
    };
    let models = [("or", text("big", 3_000_000, 15_000_000), false), ("or", text("small", 150_000, 600_000), false)];
    let picks = choose(&cap("text.complete"), &cands(&models), &Catalog::default());
    let cheapest = find(&picks, Pick::LowestCost).unwrap();
    assert_eq!((cheapest.candidate, &cheapest.by), (1, &PickedBy::Price(750_000)));
}

#[test]
fn curated_picks_come_from_the_catalog_and_only_when_the_connection_offers_the_model() {
    let models = [("or", model("flux-fill", None, None), false), ("bfl", model("flux-pro", None, None), false)];
    let c = catalog(&[
        // The first entry is for a model nobody offers, so it is skipped for the next.
        ("image.inpaint", "recommended", "or", "not-offered"),
        ("image.inpaint", "recommended", "bfl", "flux-pro"),
        ("image.inpaint", "best_result", "or", "flux-fill"),
        // A model on the wrong provider does not match.
        ("image.inpaint", "fastest", "bfl", "flux-fill"),
        // Another job's entry never leaks across.
        ("text.complete", "most_popular", "or", "flux-fill"),
    ]);
    let picks = choose(&cap("image.inpaint"), &cands(&models), &c);
    assert_eq!(find(&picks, Pick::Recommended).map(|p| p.candidate), Some(1));
    assert_eq!(find(&picks, Pick::BestResult).map(|p| p.candidate), Some(0));
    assert_eq!(find(&picks, Pick::Recommended).unwrap().by, PickedBy::Catalog);
    assert!(find(&picks, Pick::Fastest).is_none());
    assert!(find(&picks, Pick::MostPopular).is_none());
}

#[test]
fn a_model_that_disappears_stops_earning_its_pick() {
    let c = catalog(&[("image.inpaint", "recommended", "or", "flux-fill")]);
    let with = [("or", model("flux-fill", None, None), false)];
    assert!(find(&choose(&cap("image.inpaint"), &cands(&with), &c), Pick::Recommended).is_some());
    let without = [("or", model("something-else", None, None), false)];
    assert!(choose(&cap("image.inpaint"), &cands(&without), &c).is_empty());
    assert!(choose(&cap("image.inpaint"), &[], &c).is_empty());
}

#[test]
fn ties_go_to_the_id_that_sorts_first_so_the_answer_is_stable() {
    let models = [("or", model("zeta", Some(10_000), Some(500)), false), ("or", model("alpha", Some(10_000), Some(500)), false)];
    let picks = choose(&cap("image.generate"), &cands(&models), &Catalog::default());
    assert_eq!(find(&picks, Pick::LowestCost).unwrap().candidate, 1);
    assert_eq!(find(&picks, Pick::Newest).unwrap().candidate, 1);
}

#[test]
fn a_catalog_file_loads_and_a_bad_or_oversize_one_is_refused() {
    let ok = r#"{"version":2,"entries":[{"capability":"image.inpaint","pick":"recommended","provider":"or","model":"m"},
                {"capability":"image.inpaint","pick":"a_pick_from_the_future","provider":"or","model":"m"}]}"#;
    let c = Catalog::from_json(ok).unwrap();
    assert_eq!((c.version, c.entries.len()), (2, 2));
    // The unknown tag is carried but can never match a known pick.
    let models = [("or", model("m", None, None), false)];
    let picks = choose(&cap("image.inpaint"), &cands(&models), &c);
    assert_eq!(picks.len(), 1);
    assert!(matches!(Catalog::from_json("not json"), Err(CatalogError::Syntax(_))));
    assert_eq!(Catalog::from_json(&" ".repeat(MAX_CATALOG_BYTES + 1)), Err(CatalogError::TooLarge));
}

#[test]
fn which_picks_are_judgements_and_which_are_computed() {
    let curated: Vec<_> = Pick::ALL.into_iter().filter(|p| p.is_curated()).collect();
    assert_eq!(curated, [Pick::Recommended, Pick::BestResult, Pick::Fastest, Pick::MostPopular]);
    let keys: std::collections::HashSet<_> = Pick::ALL.into_iter().map(Pick::key).collect();
    assert_eq!(keys.len(), 6);
}

#[test]
fn a_newer_catalog_wins_over_the_built_in_one_and_the_rest_still_applies() {
    let newer = catalog(&[("text.complete", "recommended", "openrouter", "model-new")]);
    let built_in =
        catalog(&[("text.complete", "recommended", "openrouter", "model-old"), ("text.complete", "best_result", "openrouter", "model-best")]);
    let merged = newer.over(built_in);
    let models = [
        ("openrouter", model("model-old", None, None), false),
        ("openrouter", model("model-new", None, None), false),
        ("openrouter", model("model-best", None, None), false),
    ];
    let picks = choose(&cap("text.complete"), &cands(&models), &merged);
    let by = |p: Pick| picks.iter().find(|x| x.pick == p).map(|x| x.candidate);
    assert_eq!(by(Pick::Recommended), Some(1), "the newer entry wins");
    assert_eq!(by(Pick::BestResult), Some(2), "an entry only the old one had still counts");
}
