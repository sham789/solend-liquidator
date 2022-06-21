use hyper::Body;

use web3::types::U256;
// use rustc_hex::{FromHex, ToHex};
// use tiny_keccak::{Hasher, Keccak};

pub async fn body_to_string(body: Body) -> String {
    let body_bytes = hyper::body::to_bytes(body).await.unwrap();
    String::from_utf8(body_bytes.to_vec()).unwrap()
}

// pub fn sha3_hash_u256(input: U256) -> [u8; 32] {
//     // let mut hasher = Sha3_256::new();
//     let u256_bytes: &mut [u8] = &mut [0; 32];
//     input.to_big_endian(u256_bytes);
//     let mut h = Keccak::v256();
//     h.update(&u256_bytes);
//     let mut res: [u8; 32] = [0; 32];
//     h.finalize(&mut res);
//     res
// }
