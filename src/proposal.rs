use horizon_lib::ClusterProposal;
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::cluster::ProposalSource;
use crate::error::Result;

pub struct ProposalReader;

pub enum ProposalMsg {
    Read {
        source: ProposalSource,
        reply: RpcReplyPort<Result<ClusterProposal>>,
    },
}

#[ractor::async_trait]
impl Actor for ProposalReader {
    type Msg = ProposalMsg;
    type State = ();
    type Arguments = ();

    async fn pre_start(
        &self,
        _myself: ActorRef<Self::Msg>,
        _args: (),
    ) -> std::result::Result<Self::State, ActorProcessingErr> {
        Ok(())
    }

    async fn handle(
        &self,
        _myself: ActorRef<Self::Msg>,
        msg: Self::Msg,
        _state: &mut Self::State,
    ) -> std::result::Result<(), ActorProcessingErr> {
        match msg {
            ProposalMsg::Read { source, reply } => {
                let _ = reply.send(source.load());
            }
        }
        Ok(())
    }
}
