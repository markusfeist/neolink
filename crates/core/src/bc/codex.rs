//! Create a tokio encoder/decoder for turning a AsyncRead/Write stream into
//! a Bc packet
//!
//! BcCodex is used with a `[tokio_util::codec::Framed]` to form complete packets
//!
use crate::bc::model::*;
use crate::bc::xml::*;
use crate::{Credentials, Error, Result};
use bytes::BytesMut;
use std::io;
use tokio_util::codec::{Decoder, Encoder};

pub(crate) struct BcCodex {
    context: BcContext,
}

impl BcCodex {
    pub(crate) fn new(credentials: Credentials) -> Self {
        Self {
            context: BcContext::new(credentials),
        }
    }
}

impl Encoder<Bc> for BcCodex {
    type Error = Error;

    fn encode(&mut self, item: Bc, dst: &mut BytesMut) -> Result<()> {
        // let context = self.context.read().unwrap();
        let buf: Vec<u8> = Default::default();
        let enc_protocol: EncryptionProtocol = match self.context.get_encrypted() {
            EncryptionProtocol::Aes(_) if item.meta.msg_id == 1 => {
                // During login the encyption protocol cannot go higher than BCEncrypt
                // even if we support AES. (BUt it can go lower i.e. None)
                EncryptionProtocol::BCEncrypt
            }
            n => *n,
        };
        let buf = item.serialize(buf, &enc_protocol)?;
        dst.reserve(buf.len());
        dst.extend_from_slice(buf.as_slice());
        Ok(())
    }
}

impl Decoder for BcCodex {
    type Item = Bc;
    type Error = Error;

    fn decode_eof(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>> {
        // match self.decode(buf)? {
        //     Some(frame) => Ok(Some(frame)),
        //     None => {
        //         if buf.is_empty() {
        //             Ok(None)
        //         } else {
        //             Err(io::Error::new(
        //                 io::ErrorKind::Other,
        //                 format!("bytes remaining on BC stream: {:X?}", buf),
        //             )
        //             .into())
        //         }
        //     }
        // }
        match self.decode(buf)? {
            Some(frame) => Ok(Some(frame)),
            None => Ok(None),
        }
    }

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>> {
        // trace!("Decoding: {:X?}", src);
        let bc = Bc::deserialize(&self.context, src);
        // trace!("As: {:?}", bc);
        let bc = match bc {
            Ok(bc) => bc,
            Err(Error::NomIncomplete(_)) => return Ok(None),
            Err(e) => return Err(e),
        };
        // Update context
        if let Bc {
            meta:
                BcMeta {
                    msg_id: 1,
                    response_code,
                    ..
                },
            body:
                BcBody::ModernMsg(ModernMsg {
                    payload:
                        Some(BcPayloads::BcXml(BcXml {
                            encryption: Some(Encryption { nonce, .. }),
                            ..
                        })),
                    ..
                }),
        } = &bc
        {
            if response_code >> 8 == 0xdd {
                // Login reply has the encryption info
                // Set that the encryption type now
                let encryption_protocol_byte = (response_code & 0xff) as usize;
                match encryption_protocol_byte {
                    0x00 => self.context.set_encrypted(EncryptionProtocol::Unencrypted),
                    0x01 => self.context.set_encrypted(EncryptionProtocol::BCEncrypt),
                    0x02 => self.context.set_encrypted(EncryptionProtocol::Aes(
                        self.context.credentials.make_aeskey(nonce),
                    )),
                    _ => {
                        return Err(Error::UnknownEncryption(encryption_protocol_byte));
                    }
                }
            }
        }

        if let BcBody::ModernMsg(ModernMsg {
            extension:
                Some(Extension {
                    binary_data: Some(on_off),
                    ..
                }),
            ..
        }) = bc.body
        {
            if on_off == 0 {
                self.context.binary_off(bc.meta.msg_num);
            } else {
                self.context.binary_on(bc.meta.msg_num);
            }
        }

        Ok(Some(bc))
    }
}
