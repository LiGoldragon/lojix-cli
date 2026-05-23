use std::io::Write;
use std::path::PathBuf;

use horizon_lib::name::{ClusterName, NodeName, UserName};
use tempfile::NamedTempFile;

use lojix_cli::build::{BuildPlan, HomeBuildPlan, HomeMode, SystemAction};
use lojix_cli::cluster::{FlakeRef, ProposalSource};
use lojix_cli::request::{CommandLine, FullOs, HomeOnly, LojixRequest, OsOnly};

fn cluster_name() -> ClusterName {
    ClusterName::try_new("goldragon").unwrap()
}

fn node_name(name: &str) -> NodeName {
    NodeName::try_new(name).unwrap()
}

fn user_name(name: &str) -> UserName {
    UserName::try_new(name).unwrap()
}

fn proposal_source() -> ProposalSource {
    ProposalSource::new(PathBuf::from("/tmp/datom.nota"))
}

fn criomos_ref() -> FlakeRef {
    FlakeRef::new("github:LiGoldragon/CriomOS/abc123")
}

fn home_ref() -> FlakeRef {
    FlakeRef::new("github:LiGoldragon/CriomOS-home/main")
}

#[test]
fn inline_nota_deploy_request_decodes_after_shell_token_join() {
    let command_line = CommandLine::from_arguments([
        "(FullOs",
        "goldragon",
        "tiger",
        "[/tmp/datom.nota]",
        "[github:LiGoldragon/CriomOS/abc123]",
        "Boot",
        "None",
        "None)",
    ]);

    let request = command_line.decode_request().unwrap();

    assert_eq!(
        request,
        LojixRequest::FullOs(FullOs {
            cluster: cluster_name(),
            node: node_name("tiger"),
            source: proposal_source(),
            criomos: criomos_ref(),
            action: SystemAction::Boot,
            builder: None,
            substituters: None,
        }),
    );
}

#[test]
fn file_path_nota_os_only_request_decodes() {
    let mut file = NamedTempFile::new().unwrap();
    write!(
        file,
        "(OsOnly goldragon tiger [/tmp/datom.nota] [github:LiGoldragon/CriomOS/abc123] Build (Some prometheus) None)"
    )
    .unwrap();

    let command_line = CommandLine::from_arguments([file.path().as_os_str().to_os_string()]);
    let request = command_line.decode_request().unwrap();

    assert_eq!(
        request,
        LojixRequest::OsOnly(OsOnly {
            cluster: cluster_name(),
            node: node_name("tiger"),
            source: proposal_source(),
            criomos: criomos_ref(),
            action: SystemAction::Build,
            builder: Some(node_name("prometheus")),
            substituters: None,
        }),
    );
}

#[test]
fn system_request_decodes_named_substituters() {
    let request = LojixRequest::from_nota(
        "(FullOs goldragon zeus [/tmp/datom.nota] [github:LiGoldragon/CriomOS/abc123] Switch (Some zeus) (Some [prometheus]))",
    )
    .unwrap();

    assert_eq!(
        request,
        LojixRequest::FullOs(FullOs {
            cluster: cluster_name(),
            node: node_name("zeus"),
            source: proposal_source(),
            criomos: criomos_ref(),
            action: SystemAction::Switch,
            builder: Some(node_name("zeus")),
            substituters: Some(vec![node_name("prometheus")]),
        }),
    );
}

#[test]
fn home_only_request_decodes_user_and_mode() {
    let request = LojixRequest::from_nota(
        "(HomeOnly goldragon tiger li [/tmp/datom.nota] [github:LiGoldragon/CriomOS-home/main] Profile None None)",
    )
    .unwrap();

    assert_eq!(
        request,
        LojixRequest::HomeOnly(HomeOnly {
            cluster: cluster_name(),
            node: node_name("tiger"),
            user: user_name("li"),
            source: proposal_source(),
            home: home_ref(),
            mode: HomeMode::Profile,
            builder: None,
            substituters: None,
        }),
    );
}

#[test]
fn check_host_key_material_request_decodes() {
    let request =
        LojixRequest::from_nota("(CheckHostKeyMaterial goldragon tiger [/tmp/datom.nota])")
            .unwrap();

    assert_eq!(
        request,
        LojixRequest::CheckHostKeyMaterial(lojix_cli::check::CheckHostKeyMaterial {
            cluster: cluster_name(),
            node: node_name("tiger"),
            source: proposal_source(),
        }),
    );
}

#[test]
fn source_path_with_apostrophe_must_not_require_quote_delimiters() {
    let request = LojixRequest::from_nota(
        "(CheckHostKeyMaterial goldragon tiger [/tmp/operator's datom.nota])",
    )
    .unwrap();

    assert_eq!(
        request,
        LojixRequest::CheckHostKeyMaterial(lojix_cli::check::CheckHostKeyMaterial {
            cluster: cluster_name(),
            node: node_name("tiger"),
            source: ProposalSource::new(PathBuf::from("/tmp/operator's datom.nota")),
        }),
    );
}

#[test]
fn extra_path_arguments_are_rejected() {
    let command_line = CommandLine::from_arguments(["request.nota", "extra"]);
    let error = command_line.decode_request().unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unexpected command-line argument"),
        "unexpected error: {error}",
    );
}

#[test]
fn nota_request_rejects_trailing_tokens() {
    let error = LojixRequest::from_nota(
        "(FullOs goldragon tiger [/tmp/datom.nota] [github:LiGoldragon/CriomOS/abc123] Eval None None) trailing",
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("end of input"),
        "unexpected error: {error}",
    );
}

#[test]
fn system_records_map_to_pipeline_plans() {
    let os_only = OsOnly {
        cluster: cluster_name(),
        node: node_name("tiger"),
        source: proposal_source(),
        criomos: criomos_ref(),
        action: SystemAction::Build,
        builder: Some(node_name("prometheus")),
        substituters: Some(vec![node_name("prometheus")]),
    }
    .into_deploy_request();

    assert_eq!(os_only.plan, BuildPlan::os_only(SystemAction::Build));
    assert_eq!(os_only.cluster.as_str(), "goldragon");
    assert_eq!(os_only.node.as_str(), "tiger");
    assert_eq!(os_only.builder.as_ref().unwrap().as_str(), "prometheus");
    assert_eq!(os_only.substituters[0].as_str(), "prometheus");

    let eval = FullOs {
        cluster: cluster_name(),
        node: node_name("tiger"),
        source: proposal_source(),
        criomos: criomos_ref(),
        action: SystemAction::Eval,
        builder: None,
        substituters: None,
    }
    .into_deploy_request();

    assert_eq!(eval.plan, BuildPlan::full_os(SystemAction::Eval));
    assert!(eval.builder.is_none());
    assert!(eval.substituters.is_empty());
}

#[test]
fn home_record_maps_to_home_plan() {
    let request = HomeOnly {
        cluster: cluster_name(),
        node: node_name("tiger"),
        user: user_name("li"),
        source: proposal_source(),
        home: home_ref(),
        mode: HomeMode::Activate,
        builder: None,
        substituters: None,
    }
    .into_deploy_request();

    assert_eq!(
        request.plan,
        BuildPlan::home_only(HomeBuildPlan {
            user: user_name("li"),
            mode: HomeMode::Activate,
        })
    );
}
