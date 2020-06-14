use log;
// use lovecraft;
use simple_logger;
use sled;
use sled::IVec;
use swarm::swarm_data_service_server::{SwarmDataService, SwarmDataServiceServer};
use swarm::{
    HealthcheckRequest, HealthcheckResponse, ReadRequest, ReadResponse, WriteRequest, WriteResponse,
};
use tonic::{transport::Server, Request, Response, Status};

mod settings;
mod structures;
use settings::Settings;
// use structures::DataSubmission;

pub mod swarm {
    tonic::include_proto!("swarm"); // The string specified here must match the proto package name
}

#[derive(Debug)]
pub struct SwarmAPI {
    database: sled::Db,
}

#[tonic::async_trait]
impl SwarmDataService for SwarmAPI {
    async fn healthcheck(
        &self,
        request: Request<HealthcheckRequest>,
    ) -> Result<Response<HealthcheckResponse>, Status> {
        log::info!("Got a request: {:?}", request);

        let reply = swarm::HealthcheckResponse {
            is_healthy: true,
            message: "H E A L T H Y  ᕕ( ᐛ )ᕗ".to_string(), // message: format!("Hello there {}", request.into_inner().name).into(),
        };

        Ok(Response::new(reply))
    }

    async fn write_data(
        &self,
        request: Request<WriteRequest>,
    ) -> Result<Response<WriteResponse>, Status> {
        log::info!("User wants to write data: {:?}", &request);
        let input_data = request.into_inner();
        let db_result = &self
            .database
            .insert(input_data.key, input_data.data.as_bytes());
        // let successful = db_result.as_ref().is_ok();
        // let result = db_result.as_ref().unwrap().as_ref();
        let reply = match db_result {
            Ok(Some(x)) => WriteResponse {
                was_successful: true,
                reply: String::from_utf8(x.to_vec()).unwrap_or_default(),
            },
            Ok(None) => WriteResponse {
                was_successful: true,
                reply: input_data.data,
            },
            Err(e) => {
                log::error!("Encountered an error while writing to the database! {}", e);
                WriteResponse {
                    was_successful: false,
                    reply: "".into(),
                }
            }
        };
        Ok(Response::new(reply))
    }

    async fn read_data(
        &self,
        request: Request<ReadRequest>,
    ) -> Result<Response<ReadResponse>, Status> {
        log::info!(
            "Responding to request: {:?} from peer: {:?}",
            &request,
            &request.remote_addr()
        );
        let request_kv = request.into_inner();
        let db_result = &self.database.get(&request_kv.key).unwrap();
        let data = String::from_utf8(db_result.as_ref().unwrap().to_vec());
        let reply = ReadResponse {
            data: data.unwrap(),
        };
        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // lovecraft::invoke();
    simple_logger::init_with_level(log::Level::Info).unwrap();
    let cfg = Settings::new().unwrap();
    log::info!("Config: {:?}", cfg);
    let sled_config = sled::Config::default()
        .use_compression(cfg.db_compression_enabled)
        .path(&cfg.database_name);
    let addr = "[::1]:6666".parse()?;
    let db = sled_config.open().unwrap();
    // db.insert("k1", "Some data");
    log::info!("Database opened: {}", &cfg.database_name);
    log::info!("Database was recovered: {}", db.was_recovered());
    log::info!("Database checksum: {}", db.checksum().unwrap());
    let local_swarm = SwarmAPI { database: db };
    Server::builder()
        .add_service(SwarmDataServiceServer::new(local_swarm))
        .serve(addr)
        .await?;
    Ok(())
}

pub fn write_to_db(db: &sled::Db, key: String, value: String) -> Result<Option<IVec>, sled::Error> {
    db.insert(key, value.as_bytes())
}
