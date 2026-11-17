use super::*;

#[test]
fn exhausted_callable_regions_return_no_handle_before_adding_units_or_lambdas() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let path = root.join("script/functions.json");
    let table = fnv_script::FunctionTable::load(&path).unwrap();
    let json = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let lo = fnv_esm::LoadOrder::new(&[]).unwrap();
    let schema = fnv_esm::Schema::load_dir(root.join("fnv")).unwrap();
    let forms = fnv_sim::FormStore::default();
    let refs = fnv_sim::RefTable::default();
    let data = GameData::build(&lo, &schema, table, &json, &refs);
    let mut g = Game::new(
        &data,
        World::new(&forms, &refs, Default::default()),
        Default::default(),
    );
    g.next_compiled = TEMP_REF_BASE;
    assert!(
        g.compile_source("exhausted", "Begin Function {}\nEnd\n".into())
            .is_none()
    );
    assert!(g.compiled_files.is_empty() && g.compiled_slots.is_empty());
    g.next_lambda = FormId::FIRST_DYNAMIC.0;
    let result = g.run_query(
        "Begin Function {}\nSetFunctionValue (Begin Function {}\nEnd)\nEnd\n",
        Default::default(),
    );
    assert!(matches!(result,Err(ref e) if e.contains("runtime lambda handles exhausted")));
    assert!(g.lambdas.is_empty() && g.lambda_slots.is_empty());
    g.other.jip13.made_packages = FormId::FIRST_DYNAMIC.0 - LAMBDA_BASE;
    let result = crate::nvse_jip13::call(
        &mut g,
        &Default::default(),
        "traveltoref",
        Some(FormId::PLAYER_REF),
        &[fnv_script::vm::XVal::Ref(1)],
    );
    assert!(matches!(result, Some(Err(ref e)) if e == "runtime travel package handles exhausted"));
    assert!(g.package_overrides.is_empty());
    assert!(
        !g.extra.contains_key(&FormId::PLAYER_REF),
        "capacity failure precedes actor/package mutation"
    );
}
