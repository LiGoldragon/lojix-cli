use std::io::Write;
use std::path::PathBuf;

use horizon_lib::name::{ClusterName, NodeName};
use tempfile::NamedTempFile;

use lojix_cli::build::BuildAction;
use lojix_cli::cluster::{FlakeRef, ProposalSource};
use lojix_cli::request::{Build, CommandLine, Deploy, Eval, LojixRequest};

fn cluster_name() -> ClusterName {
    ClusterName::try_new("goldragon").unwrap()
}

fn node_name(name: &str) -> NodeName {
    NodeName::try_new(name).unwrap()
}

fn proposal_source() -> ProposalSource {
    ProposalSource::new(PathBuf::from("/tmp/datom.nota"))
}

fn criomos_ref() -> FlakeRef {
    FlakeRef::new("github:LiGoldragon/CriomOS/abc123")
}

#[test]
fn inline_nota_deploy_request_decodes_after_shell_token_join() {
    let command_line = CommandLine::from_arguments([
        "(Deploy",
        "goldragon",
        "tiger",
        "\"/tmp/datom.nota\"",
        "\"github:LiGoldragon/CriomOS/abc123\"",
        "Boot)",
    ]);

    let request = command_line.decode_request().unwrap();

    assert_eq!(
        request,
        LojixRequest::Deploy(Deploy {
            cluster: cluster_name(),
            node: node_name("tiger"),
            source: proposal_source(),
            criomos: criomos_ref(),
            action: BuildAction::Boot,
            builder: None,
        }),
    );
}

#[test]
fn file_path_nota_build_request_decodes() {
    let mut file = NamedTempFile::new().unwrap();
    write!(
        file,
        "(Build goldragon tiger \"/tmp/datom.nota\" \"github:LiGoldragon/CriomOS/abc123\" prometheus)"
    )
    .unwrap();

    let command_line = CommandLine::from_arguments([file.path().as_os_str().to_os_string()]);
    let request = command_line.decode_request().unwrap();

    assert_eq!(
        request,
        LojixRequest::Build(Build {
            cluster: cluster_name(),
            node: node_name("tiger"),
            source: proposal_source(),
            criomos: criomos_ref(),
            builder: Some(node_name("prometheus")),
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
        "(Eval goldragon tiger \"/tmp/datom.nota\" \"github:LiGoldragon/CriomOS/abc123\") trailing",
    )
    .unwrap_err();

    assert!(
        error.to_string().contains("end of input"),
        "unexpected error: {error}",
    );
}

#[test]
fn build_and_eval_records_map_to_pipeline_actions() {
    let build = LojixRequest::Build(Build {
        cluster: cluster_name(),
        node: node_name("tiger"),
        source: proposal_source(),
        criomos: criomos_ref(),
        builder: Some(node_name("prometheus")),
    })
    .into_deploy_request();

    assert_eq!(build.action, BuildAction::Build);
    assert_eq!(build.cluster.as_str(), "goldragon");
    assert_eq!(build.node.as_str(), "tiger");
    assert_eq!(build.builder.as_ref().unwrap().as_str(), "prometheus");

    let eval = LojixRequest::Eval(Eval {
        cluster: cluster_name(),
        node: node_name("tiger"),
        source: proposal_source(),
        criomos: criomos_ref(),
        builder: None,
    })
    .into_deploy_request();

    assert_eq!(eval.action, BuildAction::Eval);
    assert!(eval.builder.is_none());
}
