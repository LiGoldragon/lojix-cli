use horizon_lib::{ClusterProposal, Horizon, Viewpoint};
use ractor::{Actor, ActorProcessingErr, ActorRef, RpcReplyPort};

use crate::error::Result;

pub struct HorizonProjector;

pub struct HorizonProjection {
    proposal: ClusterProposal,
    viewpoint: Viewpoint,
}

impl HorizonProjection {
    pub fn new(proposal: ClusterProposal, viewpoint: Viewpoint) -> Self {
        Self {
            proposal,
            viewpoint,
        }
    }

    pub fn project(&self) -> Result<Horizon> {
        self.proposal.project(&self.viewpoint).map_err(Into::into)
    }
}

pub enum ProjectMsg {
    Project {
        projection: HorizonProjection,
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
            ProjectMsg::Project { projection, reply } => {
                let _ = reply.send(projection.project());
            }
        }
        Ok(())
    }
}
