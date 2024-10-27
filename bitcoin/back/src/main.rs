use tokio_postgres::{NoTls, Error};
use reqwest;
use serde::Deserialize;
use serde::Serialize;
use std::sync::Arc; // Remove the Mutex import from std
use tokio::sync::Mutex; // Use tokio's Mutex for async support
use tokio::time::{sleep, Duration};
use warp::Filter;
use std::str::FromStr;
use warp::cors;


#[derive(Debug, Deserialize, Serialize)]
struct BlockInfo {
    height: i32,
    hash: String,
    time: i64,
    txIndexes: Vec<i64> // 交易数量
}


#[derive(Debug, Deserialize, Serialize)]
struct BlockData {
    height: i32,
    hash: String,
    time:  i64,
    txIndexes: Vec<i64>, // 交易数量
    fee: f64, // 矿工费
}


#[derive(Debug, Deserialize, Serialize)]
struct SendBlockData {
    height: i32,
    hash: String,
    time: i64,
    transaction_count: i32, // 交易数量
    fee: f64, // 矿工费
    tps: f64
}

#[derive(Debug, Deserialize, Serialize)]
struct SendMarketData {
    market_volume: f64,
    timestamp: i64,
    usd: f64,  // 这是比特币价格
}



// 获取最新区块数据
async fn fetch_latest_block() -> Result<BlockData, reqwest::Error> {
    // 获取最新区块数据
    let block_response = reqwest::get("https://blockchain.info/latestblock").await?;
    let block_data: BlockInfo = block_response.json().await?;

    // 获取平均矿工费
    let fee_response = reqwest::get("https://api.blockcypher.com/v1/btc/main").await?;
    let fee_data: serde_json::Value = fee_response.json().await?;
    let average_fee = fee_data["high_fee_per_kb"].as_f64().unwrap_or(0.0);

    // 创建并返回 BlockInfo
    let block_info = BlockData {
        height: block_data.height,
        hash: block_data.hash,
        time: block_data.time,
        txIndexes: block_data.txIndexes,
        fee: average_fee, // 将平均矿工费加入 BlockInfo
    };

    println!("Fetched BlockInfo: {:?}", block_info); // 打印 BlockInfo
    Ok(block_info)
}


// 计算TPS
fn calculate_tps(transaction_count: i32, block_time_diff: i64) -> f64 {
    if block_time_diff == 0 {
        return 0.0;
    }
    transaction_count as f64 / block_time_diff as f64
}

// 保存数据到数据库
async fn save_to_db(client: &tokio_postgres::Client, block: BlockData, tps: f64) -> Result<(), Error> {
    // 打印要插入的值
    println!("Block Height: {:?}", block.height);
    println!("Block Hash: {:?}", block.hash);
    println!("Timestamp: {:?}", block.time);
    println!("Transaction Count: {:?}", block.txIndexes.len() as i32);
    println!("Miner Fee: {:?}", block.fee);
    println!("TPS: {:?}", tps);

    // 将 i64 转换为 f64
    let timestamp = block.time as f64;

    // 首先检查区块高度是否已经存在
    let exists_query = "SELECT EXISTS(SELECT 1 FROM blocks WHERE block_height = $1)";
    let exists_result = client.query_one(exists_query, &[&block.height]).await?;

    let exists: bool = exists_result.get(0);
    
    if exists {
        // 如果区块高度已经存在，则跳过存储
        println!("Block with height {} already exists. Skipping insertion.", block.height);
        return Ok(());
    }

    // 执行数据库插入
    client.execute(
        "INSERT INTO blocks (block_height, block_hash, timestamp, transaction_count, miner_fee, tps) VALUES ($1, $2, to_timestamp($3), $4, $5, $6)",
        &[&block.height, &block.hash, &timestamp, &(block.txIndexes.len() as i32), &block.fee, &tps],
    ).await?;
    
    Ok(())
}




// 链下数据获取市场交易量
#[derive(Deserialize)]
struct MarketData {
    bitcoin: BitcoinPriceData,
}

#[derive(Deserialize)]
struct BitcoinPriceData {
    usd: f64,  // 这是比特币价格
    usd_market_cap: f64,
}

async fn fetch_market_data() -> Result<(f64, f64), reqwest::Error> {
    let response = reqwest::get("https://api.coingecko.com/api/v3/simple/price?ids=bitcoin&vs_currencies=usd&include_market_cap=true")
        .await?
        .json::<MarketData>()
        .await?;
    
    // 提取比特币价格和市场交易量
    let bitcoin_price = response.bitcoin.usd;
    let market_volume = response.bitcoin.usd_market_cap;  // 此 API 返回的结构中没有交易量, 可能需要其他 API 请求来获取
    
    Ok((bitcoin_price, market_volume))
}

async fn save_market_data_to_db(client: &tokio_postgres::Client, bitcoin_price: f64, market_volume: f64) -> Result<(), Error> {
    client.execute(
        "INSERT INTO offchain_data (bitcoin_price, market_volume, timestamp) VALUES ($1, $2, NOW())",
        &[&bitcoin_price, &market_volume],
    ).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Database connection setup
    let (client, connection) = tokio_postgres::connect("host=82.180.163.150 port=5432 user=bitcoin password=12345678 dbname=bitcoin_explorer", NoTls).await?;
    let client = Arc::new(Mutex::new(client)); // Use tokio's Mutex here
    
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("connection error: {}", e);
        }
    });

    // Start the warp server in a separate task
    let client_clone = Arc::clone(&client);
    tokio::spawn(async move {
        let blocks_route = warp::path("api")
            .and(warp::path("blocks"))
            .and(warp::get())
            .and_then({
                let client = Arc::clone(&client_clone);
                move || {
                    let client = Arc::clone(&client);
                    async move {
                        let client = client.lock().await;
                        let blocks = client.query("SELECT block_height,block_hash,CAST(EXTRACT(EPOCH FROM timestamp) AS float8) as timestamp, transaction_count,miner_fee,tps FROM blocks ORDER BY timestamp ASC LIMIT 10", &[]).await.unwrap();
                        let response: Vec<SendBlockData> = blocks.iter().map(|row| {
                         let block_time: f64 = row.get(2); // 获取的是 f64，因为 EXTRACT 返回的是浮点数
                         let block_time_i64 = block_time as i64; // 转换为 i64
                        
                            SendBlockData {
                                height: row.get(0),
                                hash: row.get(1),
                                time: block_time_i64,
                                transaction_count: row.get(3), // Fill this appropriately
                                fee: row.get(4),
                                tps: row.get(5),
                            }
                        }).collect();
                        Ok::<_, warp::Rejection>(warp::reply::json(&response))
                    }
                }
            });

        let market_route = warp::path("api")
            .and(warp::path("market"))
            .and(warp::get())
            .and_then({
                let client = Arc::clone(&client_clone);
                move || {
                    let client = Arc::clone(&client);
                    async move {
                        let client = client.lock().await;
                        let market_data = client.query("SELECT bitcoin_price,market_volume,CAST(EXTRACT(EPOCH FROM timestamp) AS float8) as timestamp FROM offchain_data ORDER BY timestamp ASC LIMIT 10", &[]).await.unwrap();
                        let response: Vec<SendMarketData> = market_data.iter().map(|row| {
                            let block_time: f64 = row.get(2); // 获取的是 f64，因为 EXTRACT 返回的是浮点数
                            let block_time_i64 = block_time as i64; // 转换为 i64
                            SendMarketData {
                                market_volume: row.get(1),
                                timestamp: block_time_i64,
                                usd: row.get(0),  // 这是比特币价格
                            }
                        }).collect();
                        Ok::<_, warp::Rejection>(warp::reply::json(&response))
                    }
                }
            });
        
       // Create a CORS filter
        let cors = warp::cors()
            .allow_any_origin()  // 允许所有来源，或者您可以设置特定的来源
            .allow_headers(vec!["Content-Type"])  // 允许特定的 headers
            .allow_methods(vec!["GET", "POST"]);  // 允许的 HTTP 方法

        // Combine the routes with the CORS filter
        let routes = blocks_route.or(market_route).with(cors);

        warp::serve(routes).run(([0, 0, 0, 0], 4000)).await;
    });

    // Main loop for fetching and saving data
    loop {
        if let Ok(block) = fetch_latest_block().await {
            let tps = calculate_tps(block.txIndexes.len() as i32, 10*60);
            let client = client.lock().await; // Use .await to lock the Tokio Mutex
            save_to_db(&client, block, tps).await?;
        }

        match fetch_market_data().await {
            Ok((bitcoin_price, market_volume)) => {
                let client = client.lock().await; // Use .await to lock the Tokio Mutex
                if let Err(e) = save_market_data_to_db(&client, bitcoin_price, market_volume).await {
                    eprintln!("Error saving market data: {}", e);
                }
            }
            Err(e) => {
                eprintln!("Error fetching market data: {}", e);
            }
        }

        sleep(Duration::from_secs(60)).await;
    }
}
