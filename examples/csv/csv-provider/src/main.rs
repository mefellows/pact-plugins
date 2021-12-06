use actix_web::error::ErrorBadRequest;
use actix_web::{get, post, App, HttpResponse, HttpServer};
use anyhow::anyhow;
use csv::{ReaderBuilder, StringRecord, Writer};
use fakeit::{datetime, job, name};
use log::*;
use rand::prelude::*;

#[get("/reports/{report}.csv")]
async fn data() -> HttpResponse {
  debug!("GET request for report");
  let rows: u8 = random();
  let mut wtr = Writer::from_writer(vec![]);
  let _ = wtr.write_record(&["Name", "Occupation", "Number", "Date"]);

  for _row in 0..(rows + 1) {
    let num: u8 = random();
    let month = datetime::month().parse::<u8>().unwrap_or_default();
    let day = datetime::day().parse::<u8>().unwrap_or_default();
    let _ = wtr.write_record(&[
      name::full().as_str(),
      job::title().as_str(),
      num.to_string().as_str(),
      format!("{}-{:02}-{:02}", datetime::year(), month, day).as_str(),
    ]);
  }
  HttpResponse::Ok()
    .content_type("text/csv; charset=UTF-8")
    .body(wtr.into_inner().unwrap_or_default())
}

#[post("/reports/{report}.csv")]
async fn post_data(req_body: String) -> HttpResponse {
  debug!("POST request with report data");
  let mut rdr = ReaderBuilder::new().from_reader(req_body.as_bytes());
  let records = rdr
    .records()
    .collect::<Result<Vec<StringRecord>, csv::Error>>();

  match records {
    Ok(_data) => HttpResponse::Created().finish(),
    Err(err) => HttpResponse::from_error(ErrorBadRequest(anyhow!(
      "Error reading CSV content - {}",
      err
    ))),
  }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
  let _ = simple_log::quick();
  HttpServer::new(|| App::new().service(data).service(post_data))
    .bind("127.0.0.1:8080")?
    .run()
    .await
}
