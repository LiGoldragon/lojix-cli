use horizon_lib::{ClusterProposal, Horizon, Viewpoint};
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::error::Result;

pub struct HorizonProjector;

pub enum ProjectMsg {
    Project {
        proposal: ClusterProposal,
        viewpoint: Viewpoint,
        reply: RpcReplyPort<Result<Horizon>>,
    },
}

#[ractor::async_trait]
impl Actor for HorizonProjector {
    type Msg = ProjectMsg;
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
            ProjectMsg::Project { proposal, viewpoint, reply } => {
                let result = proposal.project(&viewpoint).map_err(Into::into);
                let _ = reply.send(result);
            }
        }
        Ok(())
    }
}
