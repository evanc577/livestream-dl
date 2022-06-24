use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockDecryptMut, KeyIvInit};
use anyhow::Result;
use log::trace;
use m3u8_rs::Key;
use reqwest::Url;
use reqwest_middleware::ClientWithMiddleware;

use crate::utils::make_absolute_url;

type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// HLS encryption methods
#[allow(dead_code)]
#[derive(Clone, Debug)]
pub enum Encryption {
    None,
    Aes128 { key: [u8; 16], iv: [u8; 16] },
    SampleAes,
}

impl Encryption {
    /// Check m3u8_key and return encryption.
    /// If encrypted, will make a query to the designated url to fetch the key
    pub async fn new(
        client: &ClientWithMiddleware,
        m3u8_key: &Key,
        base_url: &Url,
        seq: u64,
    ) -> Result<Self> {
        let encryption = match &m3u8_key {
            x if x.method == "NONE" => Self::None,
            k @ x if x.method == "AES-128" => {
                if let Some(uri) = &k.uri {
                    // Bail if keyformat exists but is not "identity"
                    if let Some(keyformat) = &k.keyformat {
                        if keyformat != "identity" {
                            return Err(anyhow::anyhow!("Invalid keyformat: {}", keyformat));
                        }
                    }

                    // Fetch key
                    let uri = make_absolute_url(base_url, uri)?;
                    trace!("Fetching encryption key from {}", uri.as_str());
                    let body = client.get(uri).send().await?.bytes().await?;
                    let mut key = [0_u8; 16];
                    key.copy_from_slice(&body[..16]);

                    // Parse IV
                    let mut iv = [0_u8; 16];
                    if let Some(iv_str) = &k.iv {
                        let iv_str = iv_str.trim_start_matches("0x");
                        hex::decode_to_slice(iv_str, &mut iv as &mut [u8])?;
                    } else {
                        let be_bytes = seq.to_be_bytes();
                        iv[8..].copy_from_slice(&be_bytes);
                    }

                    // Success
                    Self::Aes128 { key, iv }
                } else {
                    // Bail if no uri is found
                    return Err(anyhow::anyhow!("No URI found for AES-128 key"));
                }
            }
            k @ x if x.method == "SAMPLE-AES" => {
                return Err(anyhow::anyhow!(
                    "Unimplemented encryption method: {}",
                    k.method
                ))
            }
            k => return Err(anyhow::anyhow!("Invalid encryption method: {}", k.method)),
        };

        Ok(encryption)
    }

    /// Decrypt the given data
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let r = match self {
            Self::None => Vec::from(data),
            Self::Aes128 { key, iv } => {
                trace!("Decrypting segment");
                Aes128CbcDec::new(key.into(), iv.into()).decrypt_padded_vec_mut::<Pkcs7>(data)?
            }
            Self::SampleAes => unimplemented!(),
        };

        Ok(r)
    }
}
