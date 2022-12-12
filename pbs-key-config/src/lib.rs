use std::io::Write;
use std::path::Path;

use anyhow::{bail, format_err, Context, Error};
use serde::{Deserialize, Serialize};

use proxmox_lang::try_block;
use proxmox_sys::fs::{file_get_contents, replace_file, CreateOptions};

use pbs_api_types::{Fingerprint, Kdf, KeyInfo};

use pbs_tools::crypt_config::CryptConfig;

/// Key derivation function configuration
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum KeyDerivationConfig {
    Scrypt {
        n: u64,
        r: u64,
        p: u64,
        #[serde(with = "proxmox_serde::bytes_as_base64")]
        salt: Vec<u8>,
    },
    PBKDF2 {
        iter: usize,
        #[serde(with = "proxmox_serde::bytes_as_base64")]
        salt: Vec<u8>,
    },
}

impl KeyDerivationConfig {
    /// Derive a key from provided passphrase
    pub fn derive_key(&self, passphrase: &[u8]) -> Result<[u8; 32], Error> {
        let mut key = [0u8; 32];

        match self {
            KeyDerivationConfig::Scrypt { n, r, p, salt } => {
                // estimated scrypt memory usage is 128*r*n*p
                openssl::pkcs5::scrypt(passphrase, salt, *n, *r, *p, 1025 * 1024 * 1024, &mut key)?;

                Ok(key)
            }
            KeyDerivationConfig::PBKDF2 { iter, salt } => {
                openssl::pkcs5::pbkdf2_hmac(
                    passphrase,
                    salt,
                    *iter,
                    openssl::hash::MessageDigest::sha256(),
                    &mut key,
                )?;

                Ok(key)
            }
        }
    }
}

/// Encryption Key Configuration
///
/// We use this struct to store secret keys. When used with a key
/// derivation function, the key data is encrypted (AES-GCM), and you
/// need the password to restore the plain key.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct KeyConfig {
    pub kdf: Option<KeyDerivationConfig>,
    #[serde(with = "proxmox_serde::epoch_as_rfc3339")]
    pub created: i64,
    #[serde(with = "proxmox_serde::epoch_as_rfc3339")]
    pub modified: i64,
    #[serde(with = "proxmox_serde::bytes_as_base64")]
    pub data: Vec<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(default)]
    pub fingerprint: Option<Fingerprint>,
    /// Password hint
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl From<&KeyConfig> for KeyInfo {
    fn from(key_config: &KeyConfig) -> Self {
        Self {
            path: None,
            kdf: match key_config.kdf {
                Some(KeyDerivationConfig::PBKDF2 { .. }) => Kdf::PBKDF2,
                Some(KeyDerivationConfig::Scrypt { .. }) => Kdf::Scrypt,
                None => Kdf::None,
            },
            created: key_config.created,
            modified: key_config.modified,
            fingerprint: key_config.fingerprint.as_ref().map(|fp| fp.signature()),
            hint: key_config.hint.clone(),
        }
    }
}

impl KeyConfig {
    /// Creates a new key using random data, protected by passphrase.
    pub fn new(passphrase: &[u8], kdf: Kdf) -> Result<([u8; 32], Self), Error> {
        let mut key = [0u8; 32];
        proxmox_sys::linux::fill_with_random_data(&mut key)?;
        let key_config = Self::with_key(&key, passphrase, kdf)?;
        Ok((key, key_config))
    }

    /// Creates a new, unencrypted key.
    pub fn without_password(raw_key: [u8; 32]) -> Result<Self, Error> {
        // always compute fingerprint
        let crypt_config = CryptConfig::new(raw_key)?;
        let fingerprint = Some(Fingerprint::new(crypt_config.fingerprint()));

        let created = proxmox_time::epoch_i64();
        Ok(Self {
            kdf: None,
            created,
            modified: created,
            data: raw_key.to_vec(),
            fingerprint,
            hint: None,
        })
    }

    /// Creates a new instance, protect raw_key with passphrase.
    pub fn with_key(raw_key: &[u8; 32], passphrase: &[u8], kdf: Kdf) -> Result<Self, Error> {
        if raw_key.len() != 32 {
            bail!("got strange key length ({} != 32)", raw_key.len())
        }

        let salt = proxmox_sys::linux::random_data(32)?;

        let kdf = match kdf {
            Kdf::Scrypt => KeyDerivationConfig::Scrypt {
                n: 65536,
                r: 8,
                p: 1,
                salt,
            },
            Kdf::PBKDF2 => KeyDerivationConfig::PBKDF2 { iter: 65535, salt },
            Kdf::None => {
                bail!("No key derivation function specified");
            }
        };

        let derived_key = kdf.derive_key(passphrase)?;

        let cipher = openssl::symm::Cipher::aes_256_gcm();

        let iv = proxmox_sys::linux::random_data(16)?;
        let mut tag = [0u8; 16];

        let encrypted_key =
            openssl::symm::encrypt_aead(cipher, &derived_key, Some(&iv), b"", raw_key, &mut tag)?;

        let mut enc_data = vec![];
        enc_data.extend_from_slice(&iv);
        enc_data.extend_from_slice(&tag);
        enc_data.extend_from_slice(&encrypted_key);

        let created = proxmox_time::epoch_i64();

        // always compute fingerprint
        let crypt_config = CryptConfig::new(*raw_key)?;
        let fingerprint = Some(Fingerprint::new(crypt_config.fingerprint()));

        Ok(Self {
            kdf: Some(kdf),
            created,
            modified: created,
            data: enc_data,
            fingerprint,
            hint: None,
        })
    }

    /// Loads a KeyConfig from path
    pub fn load<P: AsRef<Path>>(path: P) -> Result<KeyConfig, Error> {
        let keydata = file_get_contents(path)?;
        let key_config: KeyConfig = serde_json::from_reader(&keydata[..])?;
        Ok(key_config)
    }

    /// Decrypt key to get raw key data.
    pub fn decrypt(
        &self,
        passphrase: &dyn Fn() -> Result<Vec<u8>, Error>,
    ) -> Result<([u8; 32], i64, Fingerprint), Error> {
        let raw_data = &self.data;

        let key = if let Some(ref kdf) = self.kdf {
            let passphrase = passphrase()?;
            if passphrase.len() < 5 {
                bail!("Passphrase is too short!");
            }

            let derived_key = kdf.derive_key(&passphrase)?;

            if raw_data.len() < 32 {
                bail!("Unable to decrypt key - short data");
            }
            let iv = &raw_data[0..16];
            let tag = &raw_data[16..32];
            let enc_data = &raw_data[32..];

            let cipher = openssl::symm::Cipher::aes_256_gcm();

            openssl::symm::decrypt_aead(cipher, &derived_key, Some(iv), b"", enc_data, tag)
                .map_err(|err| match self.hint {
                    Some(ref hint) => {
                        format_err!("Unable to decrypt key (password hint: {})", hint)
                    }
                    None => {
                        format_err!("Unable to decrypt key (wrong password?) - {}", err)
                    }
                })?
        } else {
            raw_data.clone()
        };

        let mut result = [0u8; 32];
        result.copy_from_slice(&key);

        let crypt_config = CryptConfig::new(result)?;
        let fingerprint = Fingerprint::new(crypt_config.fingerprint());
        if let Some(ref stored_fingerprint) = self.fingerprint {
            if &fingerprint != stored_fingerprint {
                bail!(
                    "KeyConfig contains wrong fingerprint {}, contained key has fingerprint {}",
                    stored_fingerprint,
                    fingerprint
                );
            }
        }

        Ok((result, self.created, fingerprint))
    }

    /// Store a KeyConfig to path
    pub fn store<P: AsRef<Path>>(&self, path: P, replace: bool) -> Result<(), Error> {
        let path: &Path = path.as_ref();

        let data = serde_json::to_string(self)?;

        try_block!({
            if replace {
                let mode = nix::sys::stat::Mode::S_IRUSR | nix::sys::stat::Mode::S_IWUSR;
                replace_file(path, data.as_bytes(), CreateOptions::new().perm(mode), true)?;
            } else {
                use std::os::unix::fs::OpenOptionsExt;

                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .mode(0o0600)
                    .create_new(true)
                    .open(path)?;

                file.write_all(data.as_bytes())?;
            }

            Ok(())
        })
        .map_err(|err: Error| format_err!("Unable to store key file {:?} - {}", path, err))?;

        Ok(())
    }
}

/// Loads a KeyConfig from path and decrypt it.
pub fn load_and_decrypt_key(
    path: &std::path::Path,
    passphrase: &dyn Fn() -> Result<Vec<u8>, Error>,
) -> Result<([u8; 32], i64, Fingerprint), Error> {
    decrypt_key(&file_get_contents(path)?, passphrase)
        .with_context(|| format!("failed to load decryption key from {:?}", path))
}

/// Decrypt a KeyConfig from raw keydata.
pub fn decrypt_key(
    mut keydata: &[u8],
    passphrase: &dyn Fn() -> Result<Vec<u8>, Error>,
) -> Result<([u8; 32], i64, Fingerprint), Error> {
    let key_config: KeyConfig = serde_json::from_reader(&mut keydata)?;
    key_config.decrypt(passphrase)
}

/// RSA encrypt a KeyConfig using a public key
pub fn rsa_encrypt_key_config(
    rsa: openssl::rsa::Rsa<openssl::pkey::Public>,
    key: &KeyConfig,
) -> Result<Vec<u8>, Error> {
    let data = serde_json::to_string(key)?.as_bytes().to_vec();

    let mut buffer = vec![0u8; rsa.size() as usize];
    let len = rsa.public_encrypt(&data, &mut buffer, openssl::rsa::Padding::PKCS1)?;
    if len != buffer.len() {
        bail!("got unexpected length from rsa.public_encrypt().");
    }
    Ok(buffer)
}

/// RSA deccrypt a KeyConfig using a private key
pub fn rsa_decrypt_key_config(
    rsa: openssl::rsa::Rsa<openssl::pkey::Private>,
    key: &[u8],
    passphrase: &dyn Fn() -> Result<Vec<u8>, Error>,
) -> Result<([u8; 32], i64, Fingerprint), Error> {
    let mut buffer = vec![0u8; rsa.size() as usize];
    let decrypted = rsa
        .private_decrypt(key, &mut buffer, openssl::rsa::Padding::PKCS1)
        .map_err(|err| format_err!("failed to decrypt KeyConfig using RSA - {}", err))?;
    decrypt_key(&buffer[..decrypted], passphrase)
}

#[test]
fn encrypt_decrypt_test() -> Result<(), Error> {
    use openssl::bn::BigNum;

    // hard-coded RSA key to avoid RNG load
    let n = BigNum::from_dec_str("763297775503047285270249195475255390818773358873206395804367739392073195066702037300664538507287660511788773520960052020490020500131532096848837840341808263208238432840771609456175669299183585737297710099814398628316822920397690811697390531460556770185920171717205255045261184699028939408338685227311360280561223048029934992213591164033485740834987719043448066906674761591422028943934961140687347873900379335604823288576732038392698785999130614670054444889172406669687648738432457362496418067100853836965448514921778954255248154805294695304544857640397043149235605321195443660560591215396460879078079707598866525981810195613239506906019678159905700662365794059211182357495974678620101388841934629146959674859076053348229540838236896752745251734302737503775744293828247434369307467826891918526442390310055226655466835862319406221740752718258752129277114593279326227799698036058425261999904258111333276380007458144919278763944469942242338999234161147188585579934794573969834141472487673642778646170134259790130811461184848743147137198639341697548363179639042991358823669057297753206096865332303845149920379065177826748710006313272747133642274061146677367740923397358666767242901746171920401890395722806446280380164886804469750825832083").expect("converting to bignum failed");
    let e = BigNum::from_dec_str("65537").expect("converting to bignum failed");
    let d = BigNum::from_dec_str("19834537920284564853674022001226176519590018312725185651690468898251379391772488358073023011091610629897253174637151053464371346136136825929376853412608136964518211867003891708559549030570664609466682947037305962494828103719078802086159819263581307957743290849968728341884428605863043529798446388179368090663224786773806846388143274064254180335413340334940446739125488182098535411927937482988091512111514808559058456451259207186517021416246081401087976557460070014777577029793101223558164090029643622447657946212243306210181845486266030884899215596710196751196243890657122549917370139613045651724521564033154854414253451612565268626314358200247667906740226693180923631251719053819020017537699856142036238058103150388959616397059243552685604990510867544536282659146915388522812398795915840913802745825670833498941795568293354230962683054249223513028733221781409833526268687556063636480230666207346771664323325175723577540510559973905170578206847160551684632855673373061549848844186260938182413805301541655002820734307939021848604620517318497220269398148326924299176570233223593669359192722153811016413065311904503101005564780859010942238851216519088762587394817890851764597501374473176420295837906296738426781972820833509964922715585").expect("converting to bignum failed");
    let p = BigNum::from_dec_str("29509637001892646371585718218450720181675215968655693119622290166463846337874978909899277049204111617901784460858811114760264767076166751445502024396748257412446297522757119324882999179307561418697097464139952930737249422485899639568595470472222197161276683797577982497955467948265299386993875583089675892019886767032750524889582030672594405810531152141432362873209548569385820623081973262550874468619670422387868884561012170536839449407663630232422905779693831681822257822783504983493794208329832510955061326579888576047912149807967610736616238778237407615015312695567289456675371922184276823263863231190560557676339").expect("converting to bignum failed");
    let q = BigNum::from_dec_str("25866050993920799422553175902510303878636288340476152724026122959148470649546748310678170203350410878157245623372422271950639190884394436256045773535202161325882791039345330048364703416719823181485853395688815455066122599160191671526435061804017559815713791273329637690511813515454721229797045837580571003198471014420883727461348135261877384657284061678787895040009197824032371314493780688519536250146270701914875469190776765810821706480996720025323321483843112182646061748043938180130013308823672610860230340094502643614566152670758944502783858455501528490806234504795239898001698524105646533910560293336400403204897").expect("converting to bignum failed");
    let dmp1 = BigNum::from_dec_str("21607770579166338313924278588690558922108583912962897316392792781303188398339022047518905458553289108745759383366535358272664077428797321640702979183532285223743426240475893650342331272664468275332046219832278884297711602396407401980831582724583041600551528176116883960387063733484217876666037528133838392148714866050744345765006980605100330287254053877398358630385580919903058731105447806937933747350668236714360621211130384969129674812319182867594036995223272269821421615266717078107026511273509659211002684589097654567453625356436054504001404801715927134738465685565147724902539753143706245247513141254140715042985").expect("converting to bignum failed");
    let dmq1 = BigNum::from_dec_str("294824909477987048059069264677589712640818276551195295555907561384926187881828905626998384758270243160099828809057470393016578048898219996082612765778049262408020582364022789357590879232947921274546172186391582540158896220038500063021605980859684104892476037676079761887366292263067835858498149757735119694054623308549371262243115446856316376077501168409517640338844786525965200908293851935915491689568704919822573134038943559526432621897623477713604851434011395096458613085567264607124524187730342254186063812054159860538030670385536895853938115358646898433438472543479930479076991585011794266310458811393428158049").expect("converting to bignum failed");
    let iqmp = BigNum::from_dec_str("19428066064824171668277167138275898936765006396600005071379051329779053619544399695639107933588871625444213173194462077344726482973273922001955114108600584475883837715007613468112455972196002915686862701860412263935895363086514864873592142686096117947515832613228762197577036084559813332497101195090727973644165586960538914545531208630624795512138060798977135902359295307626262953373309121954863020224150277262533638440848025788447039555055470985052690506486164836957350781708784380677438638580158751807723730202286612196281022183410822668814233870246463721184575820166925259871133457423401827024362448849298618281053").expect("converting to bignum failed");
    let public =
        openssl::rsa::Rsa::from_public_components(n.to_owned().unwrap(), e.to_owned().unwrap())
            .expect("creating hard-coded RSA public key instance failed");
    let private = openssl::rsa::Rsa::from_private_components(n, e, d, p, q, dmp1, dmq1, iqmp)
        .expect("creating hard-coded RSA key instance failed");

    let passphrase = || -> Result<Vec<u8>, Error> { Ok(Vec::new()) };

    let key = KeyConfig {
        kdf: None,
        created: proxmox_time::epoch_i64(),
        modified: proxmox_time::epoch_i64(),
        data: (0u8..32u8).collect(),
        fingerprint: Some(Fingerprint::new([
            14, 171, 212, 70, 11, 110, 185, 202, 52, 80, 35, 222, 226, 183, 120, 199, 144, 229, 74,
            22, 131, 185, 101, 156, 10, 87, 174, 25, 144, 144, 21, 155,
        ])),
        hint: None,
    };

    let encrypted = rsa_encrypt_key_config(public, &key).expect("encryption failed");
    let (decrypted, created, fingerprint) =
        rsa_decrypt_key_config(private, &encrypted, &passphrase).expect("decryption failed");

    assert_eq!(key.created, created);
    assert_eq!(key.data, decrypted);
    assert_eq!(key.fingerprint, Some(fingerprint));

    Ok(())
}

#[test]
fn fingerprint_checks() -> Result<(), Error> {
    let key = KeyConfig {
        kdf: None,
        created: proxmox_time::epoch_i64(),
        modified: proxmox_time::epoch_i64(),
        data: (0u8..32u8).collect(),
        fingerprint: Some(Fingerprint::new([0u8; 32])), // wrong FP
        hint: None,
    };

    let expected_fingerprint = Fingerprint::new([
        14, 171, 212, 70, 11, 110, 185, 202, 52, 80, 35, 222, 226, 183, 120, 199, 144, 229, 74, 22,
        131, 185, 101, 156, 10, 87, 174, 25, 144, 144, 21, 155,
    ]);

    let data = serde_json::to_vec(&key).expect("encoding KeyConfig failed");
    decrypt_key(&data, &{ || Ok(Vec::new()) })
        .expect_err("decoding KeyConfig with wrong fingerprint worked");

    let key = KeyConfig {
        kdf: None,
        created: proxmox_time::epoch_i64(),
        modified: proxmox_time::epoch_i64(),
        data: (0u8..32u8).collect(),
        fingerprint: None,
        hint: None,
    };

    let data = serde_json::to_vec(&key).expect("encoding KeyConfig failed");
    let (key_data, created, fingerprint) = decrypt_key(&data, &{ || Ok(Vec::new()) })
        .expect("decoding KeyConfig without fingerprint failed");

    assert_eq!(key.data, key_data);
    assert_eq!(key.created, created);
    assert_eq!(expected_fingerprint, fingerprint);

    Ok(())
}
