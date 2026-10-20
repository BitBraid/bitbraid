//! X.509 signatures from a certificate authority of the test's own: a root,
//! an intermediate below it and the signer's certificates below that, made
//! with openssl. The signer's key is made by gpgsm, which asks for a
//! certificate; the intermediate issues two, one valid now and one that was
//! valid in 2020 only. Commits are signed through `commit.create` with
//! gpgsm, and judged by `sign.verify` held against `%G?` and `git
//! verify-commit` (the harness of `signing.rs`), in a certificate store
//! made for each case: the chain trusted at its root, with and without the
//! intermediate in the store; the intermediate nowhere; the root not
//! trusted; the certificate expired; the certificate revoked by a
//! revocation list loaded into the store's dirmngr; no revocation list to
//! be had; and a list that revokes nothing.
//!
//! Every gpgsm, gpg-agent and dirmngr runs with a GnuPG home of the test's
//! own, named to them by a short link in `/tmp` (a socket's path is at
//! most 108 bytes, and gpg keeps them in the home where the system has no
//! runtime directory for the user), and is ended with it. Nothing goes to
//! the network: the distribution points the certificates name are a closed
//! port of the loopback interface, and every dirmngr is told to fetch
//! nothing over HTTP or LDAP.
//!
//! Sources: gpgsm(1) (`--generate-key` and its parameters, `--import`,
//! `--include-certs`, `--disable-crl-checks`, `--faked-system-time`,
//! `--call-dirmngr loadcrl`, `trustlist.txt`), dirmngr(8)
//! (`--disable-http`, `--disable-ldap`, the CRL cache), gpg-agent(1),
//! openssl-req(1), openssl-ca(1) (`-selfsign`, `-startdate`, `-enddate`,
//! `-revoke`, `-gencrl`), openssl-crl(1), x509v3_config(5); git-log(1)
//! (`%G?`), git-verify-commit(1).
#![allow(clippy::print_stdout)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use bitbraid_testkit::scratch_root;
use serde_json::{Value, json};

use crate::signing::{Harness, for_gpg, missing, private, script};

/// The openssl configuration of both authorities: one database each, and
/// the extensions of every kind of certificate made here. Each authority's
/// own name follows it, the last section's (see [`ca_config`]).
const CA_CONFIG: &str = "\
[ ca ]
default_ca = authority
[ authority ]
dir = .
database = index.txt
new_certs_dir = .
serial = serial
crlnumber = crlnumber
default_md = sha256
default_days = 3650
default_crl_days = 3650
policy = anything
copy_extensions = none
unique_subject = no
email_in_dn = no
[ anything ]
commonName = supplied
organizationName = optional
[ root_ext ]
basicConstraints = critical,CA:TRUE
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
[ intermediate_ext ]
basicConstraints = critical,CA:TRUE,pathlen:0
keyUsage = critical,keyCertSign,cRLSign
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid
crlDistributionPoints = URI:http://127.0.0.1:9/root.crl
[ leaf_ext ]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature,nonRepudiation
extendedKeyUsage = emailProtection
subjectKeyIdentifier = hash
authorityKeyIdentifier = keyid
subjectAltName = email:ada@fixture.invalid
crlDistributionPoints = URI:http://127.0.0.1:9/intermediate.crl
[ req ]
distinguished_name = req_dn
prompt = no
[ req_dn ]
";

/// The configuration of the authority named `name`. Its name is in the
/// file, not only in `-subj`: LibreSSL (macOS's openssl) takes the file's
/// over `-subj` where the file says `prompt = no`.
fn ca_config(name: &str) -> String {
    format!("{CA_CONFIG}CN = {name}\nO = BitBraid Test\n")
}

/// When everything above the signer's certificates is valid: from before
/// 2020, when the expired one signed, for as long as the test may be run.
const SINCE: &str = "20190101000000Z";
const UNTIL: &str = "20990101000000Z";

/// The GnuPG homes of the test, each by its short link; ended and taken
/// away when this is dropped, before the harness takes the scratch away.
struct Homes {
    links: Vec<PathBuf>,
}

impl Homes {
    /// A new GnuPG home in `dir` and the short link it is known by. Its
    /// agent asks nobody (no pinentry, no marking of roots as trusted),
    /// and its dirmngr fetches nothing.
    fn make(&mut self, dir: &Path) -> PathBuf {
        assert!(dir.starts_with(scratch_root()), "{dir:?}");
        fs::create_dir_all(dir).unwrap();
        private(dir);
        fs::write(
            dir.join("gpg-agent.conf"),
            "pinentry-program /bin/false\nno-allow-mark-trusted\n",
        )
        .unwrap();
        fs::write(dir.join("dirmngr.conf"), "disable-http\ndisable-ldap\n").unwrap();
        // On Windows a junction: MSYS2's sockets are as short as macOS's.
        #[cfg(windows)]
        let link = crate::signing::junction(dir, "bbx");
        #[cfg(unix)]
        let link = {
            let key = bitbraid_testkit::fnv1a64(dir.as_os_str().as_encoded_bytes());
            let link = PathBuf::from(format!("/tmp/bbx-{key:016x}"));
            let _ = fs::remove_file(&link);
            std::os::unix::fs::symlink(dir, &link).unwrap();
            link
        };
        self.links.push(link.clone());
        link
    }
}

impl Drop for Homes {
    fn drop(&mut self) {
        for link in &self.links {
            // Only ever a home made here: gpgconf with another would talk
            // about the developer's own agent.
            let made_here = if cfg!(windows) {
                let real = |path: &Path| fs::canonicalize(path).ok();
                real(link)
                    .zip(real(&scratch_root()))
                    .is_some_and(|(to, root)| to.starts_with(root))
            } else {
                fs::read_link(link).is_ok_and(|to| to.starts_with(scratch_root()))
            };
            if !made_here {
                continue;
            }
            for action in [&["--kill", "all"][..], &["--remove-socketdir"]] {
                let _ = Command::new("gpgconf")
                    .args(action)
                    .env("GNUPGHOME", for_gpg(link))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            if cfg!(windows) {
                let _ = fs::remove_dir(link);
            } else {
                let _ = fs::remove_file(link);
            }
        }
    }
}

/// openssl in `dir`; false, with a line saying why, where it failed.
fn openssl(dir: &Path, args: &[&str]) -> bool {
    let out = Command::new("openssl")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    if !out.status.success() {
        println!(
            "skipped: openssl {} failed here: {}",
            args.first().unwrap_or(&""),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    out.status.success()
}

/// A certificate's SHA-1 fingerprint, as gpgsm names it.
fn fingerprint(dir: &Path, file: &str) -> String {
    let out = Command::new("openssl")
        .args(["x509", "-noout", "-fingerprint", "-sha1", "-in", file])
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout);
    text.trim()
        .rsplit('=')
        .next()
        .unwrap()
        .replace(':', "")
        .to_uppercase()
}

/// The authorities: a root that signs itself and an intermediate below it,
/// each with its own database, in `ca/root` and `ca/intermediate`.
struct Authority {
    dir: PathBuf,
    root: PathBuf,
    intermediate: PathBuf,
}

impl Authority {
    fn make(dir: &Path) -> Option<Self> {
        let (root, intermediate) = (dir.join("root"), dir.join("intermediate"));
        for (db, name) in [(&root, "Test Root"), (&intermediate, "Test Intermediate")] {
            fs::create_dir_all(db).unwrap();
            fs::write(db.join("ca.cnf"), ca_config(name)).unwrap();
            fs::write(db.join("index.txt"), "").unwrap();
            fs::write(db.join("serial"), "1000\n").unwrap();
            fs::write(db.join("crlnumber"), "1000\n").unwrap();
        }
        let ok = openssl(
            &root,
            &[
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-config",
                "ca.cnf",
                "-subj",
                "/CN=Test Root/O=BitBraid Test",
                "-keyout",
                "ca.key",
                "-out",
                "ca.csr",
            ],
        ) && openssl(
            &root,
            &[
                "ca",
                "-batch",
                "-notext",
                "-config",
                "ca.cnf",
                "-selfsign",
                "-keyfile",
                "ca.key",
                "-in",
                "ca.csr",
                "-startdate",
                SINCE,
                "-enddate",
                UNTIL,
                "-extensions",
                "root_ext",
                "-out",
                "ca.pem",
            ],
        ) && openssl(
            &intermediate,
            &[
                "req",
                "-new",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-config",
                "ca.cnf",
                "-subj",
                "/CN=Test Intermediate/O=BitBraid Test",
                "-keyout",
                "ca.key",
                "-out",
                "ca.csr",
            ],
        );
        let intermediate_csr = intermediate.join("ca.csr");
        let intermediate_pem = intermediate.join("ca.pem");
        let ok = ok
            && openssl(
                &root,
                &[
                    "ca",
                    "-batch",
                    "-notext",
                    "-config",
                    "ca.cnf",
                    "-cert",
                    "ca.pem",
                    "-keyfile",
                    "ca.key",
                    "-in",
                    intermediate_csr.to_str().unwrap(),
                    "-startdate",
                    SINCE,
                    "-enddate",
                    UNTIL,
                    "-extensions",
                    "intermediate_ext",
                    "-out",
                    intermediate_pem.to_str().unwrap(),
                ],
            );
        ok.then(|| Self {
            dir: dir.to_path_buf(),
            root,
            intermediate,
        })
    }

    /// Have the intermediate issue a certificate for a request, valid from
    /// `since` until `until`; its file.
    fn issue(&self, request: &Path, name: &str, since: &str, until: &str) -> Option<PathBuf> {
        let out = self.dir.join(format!("{name}.pem"));
        openssl(
            &self.intermediate,
            &[
                "ca",
                "-batch",
                "-notext",
                "-config",
                "ca.cnf",
                "-cert",
                "ca.pem",
                "-keyfile",
                "ca.key",
                "-in",
                request.to_str().unwrap(),
                "-startdate",
                since,
                "-enddate",
                until,
                "-extensions",
                "leaf_ext",
                "-out",
                out.to_str().unwrap(),
            ],
        )
        .then_some(out)
    }

    /// The revocation list an authority issues now, in the form dirmngr
    /// loads (DER); `from` is `root` or `intermediate`.
    fn revocation_list(&self, from: &Path, name: &str) -> Option<PathBuf> {
        let pem = self.dir.join(format!("{name}.crl.pem"));
        let der = self.dir.join(format!("{name}.crl"));
        (openssl(
            from,
            &[
                "ca",
                "-batch",
                "-config",
                "ca.cnf",
                "-cert",
                "ca.pem",
                "-keyfile",
                "ca.key",
                "-gencrl",
                "-out",
                pem.to_str().unwrap(),
            ],
        ) && openssl(
            from,
            &[
                "crl",
                "-in",
                pem.to_str().unwrap(),
                "-outform",
                "DER",
                "-out",
                der.to_str().unwrap(),
            ],
        ))
        .then_some(der)
    }

    /// Have the intermediate revoke a certificate it issued.
    fn revoke(&self, certificate: &Path) -> bool {
        openssl(
            &self.intermediate,
            &[
                "ca",
                "-batch",
                "-config",
                "ca.cnf",
                "-cert",
                "ca.pem",
                "-keyfile",
                "ca.key",
                "-revoke",
                certificate.to_str().unwrap(),
            ],
        )
    }
}

/// How a store checks revocation lists.
#[derive(Clone, Copy)]
enum Lists<'a> {
    /// It does not.
    Off,
    /// It does, and has these loaded into its dirmngr, in this order.
    Loaded(&'a [&'a Path]),
}

/// One case: what the store has, and the commit asked about.
struct Store<'a> {
    name: &'a str,
    intermediate: bool,
    trusted: bool,
    lists: Lists<'a>,
}

/// The harness, the homes and the authorities. The homes come first, so
/// that their agents are ended before the harness takes the scratch away.
struct Chains {
    homes: Homes,
    h: Harness,
    authority: Authority,
    root_fingerprint: String,
}

impl Chains {
    /// The script that is gpgsm with this home, with `extra` before what
    /// git passes.
    fn wrapper(&self, name: &str, home: &Path, extra: &str) -> PathBuf {
        script(
            &self.h.keys.join(format!("gpgsm-ca-{name}.sh")),
            &format!(
                "#!/bin/sh\n# gpgsm with a store of its own, and no window to ask in.\nunset DISPLAY WAYLAND_DISPLAY\nGNUPGHOME='{}'\nexport GNUPGHOME\nexec gpgsm {extra} \"$@\"\n",
                for_gpg(home).display()
            ),
        )
    }

    fn import(&self, wrapper: &Path, file: &Path) {
        let out = self
            .h
            .gpg(wrapper, &["--batch", "--import", file.to_str().unwrap()]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn trust_root(&self, home: &Path) {
        fs::write(
            home.join("trustlist.txt"),
            format!("{} S relax\n", self.root_fingerprint),
        )
        .unwrap();
    }

    /// A store for one case, and the script that is gpgsm with it; git
    /// runs that from now on.
    fn store(&mut self, store: &Store<'_>, certificates: &[&Path]) {
        let dir = self.h.keys.join(format!("ca-store-{}", store.name));
        let home = self.homes.make(&dir);
        let wrapper = self.wrapper(store.name, &home, "");
        self.import(&wrapper, &self.authority.root.join("ca.pem"));
        if store.intermediate {
            self.import(&wrapper, &self.authority.intermediate.join("ca.pem"));
        }
        for certificate in certificates {
            self.import(&wrapper, certificate);
        }
        if store.trusted {
            self.trust_root(&dir);
        }
        match store.lists {
            Lists::Off => fs::write(dir.join("gpgsm.conf"), "disable-crl-checks\n").unwrap(),
            Lists::Loaded(lists) => {
                for list in lists {
                    let out = self.h.gpg(
                        &wrapper,
                        &[
                            "--batch",
                            "--call-dirmngr",
                            "loadcrl",
                            list.to_str().unwrap(),
                        ],
                    );
                    assert!(
                        out.status.success(),
                        "loading {list:?}: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                }
            }
        }
        self.h.set("gpg.x509.program", wrapper.to_str().unwrap());
    }

    /// The verdict on one commit in this store, held against git's.
    fn judged(&mut self, store: &Store<'_>, certificates: &[&Path], id: &str) -> Value {
        self.store(store, certificates);
        let verdict = self.h.verified(&[id]).remove(0);
        println!(
            "case {}: state {}, chain {}, trust {}, reason {}",
            store.name, verdict["state"], verdict["chain"], verdict["trust"], verdict["reason"]
        );
        verdict
    }

    /// The signed tag in the store in use, held against git: a chain that
    /// does not hold is what `sign.verify` says of the commit, and `git
    /// verify-tag` refuses the tag.
    fn tag_judged(&mut self, fault: &str) {
        let details = self.h.tag_details("certified");
        let signature = &details["signature"];
        assert_eq!(signature["state"], "untrusted", "{details}");
        assert_eq!(signature["format"], "x509", "{details}");
        assert_eq!(signature["chain"], fault, "{details}");
        assert!(signature["reason"].is_string(), "{details}");
        assert!(
            self.h
                .git()
                .run(["verify-tag", "refs/tags/certified"])
                .is_err()
        );
    }

    /// A signed commit made by the command.
    fn commit(&mut self, file: &str) -> String {
        self.h.copy.write(file, file).unwrap();
        let out = self.h.run(
            "commit.create",
            json!({ "message": file, "stage_all": true, "sign": true }),
        );
        let id = out["id"].as_str().unwrap().to_owned();
        let raw = self.h.git().run(["cat-file", "commit", &id]).unwrap();
        assert!(
            raw.contains("gpgsig -----BEGIN SIGNED MESSAGE-----"),
            "{raw}"
        );
        id
    }
}

#[test]
fn x509_chains_from_an_authority_are_judged_as_git_judges_them() {
    if missing("gpgsm") || missing("gpgconf") || missing("dirmngr") || missing("openssl") {
        return;
    }
    let h = Harness::new();
    let mut homes = Homes { links: Vec::new() };
    let ca_dir = h.keys.join("ca");
    let Some(authority) = Authority::make(&ca_dir) else {
        return;
    };
    let root_fingerprint = fingerprint(&authority.root, "ca.pem");
    assert_eq!(root_fingerprint.len(), 40, "{root_fingerprint}");

    // The signer's store: the key is gpgsm's, the certificates the
    // intermediate's. It checks no revocation lists, so that signing does
    // not depend on them.
    let signer_dir = h.keys.join("ca-signer");
    let signer_home = homes.make(&signer_dir);
    fs::write(signer_dir.join("gpgsm.conf"), "disable-crl-checks\n").unwrap();
    let mut c = Chains {
        homes,
        h,
        authority,
        root_fingerprint,
    };
    let signer = c.wrapper("signer", &signer_home, "");
    let params = c.h.keys.join("ca-request-params");
    fs::write(
        &params,
        "Key-Type: RSA\nKey-Length: 2048\nKey-Usage: sign\n\
         Name-DN: CN=Ada Fixture,O=BitBraid Test\nName-Email: ada@fixture.invalid\n",
    )
    .unwrap();
    // No serial: a request, for an authority to sign. The passphrase is
    // read from standard input, which is empty.
    let made = c.h.gpg(
        &signer,
        &[
            "--batch",
            "--armor",
            "--pinentry-mode",
            "loopback",
            "--passphrase-fd",
            "0",
            "--generate-key",
            params.to_str().unwrap(),
        ],
    );
    if !made.status.success() {
        println!(
            "skipped: gpgsm could not make a key here: {}",
            String::from_utf8_lossy(&made.stderr).trim()
        );
        return;
    }
    let request = c.h.keys.join("ca-leaf.csr");
    fs::write(&request, &made.stdout).unwrap();
    let Some(leaf) = c.authority.issue(&request, "leaf", SINCE, UNTIL) else {
        return;
    };
    let Some(old) = c
        .authority
        .issue(&request, "old", "20200101000000Z", "20210101000000Z")
    else {
        return;
    };
    for certificate in [
        c.authority.root.join("ca.pem"),
        c.authority.intermediate.join("ca.pem"),
        leaf.clone(),
        old.clone(),
    ] {
        c.import(&signer, &certificate);
    }
    c.trust_root(&signer_dir);
    let leaf_print = fingerprint(&c.authority.dir, "leaf.pem");
    let old_print = fingerprint(&c.authority.dir, "old.pem");

    // Both certificates are there to choose from, and the expired one
    // cannot sign.
    c.h.set("gpg.format", "x509");
    c.h.set("user.signingkey", &leaf_print);
    c.h.set("gpg.x509.program", signer.to_str().unwrap());
    let status = c.h.run("sign.status", json!({}));
    assert_eq!(status["can_sign"], true, "{status}");
    let no_files = c.h.keys.join("ca-no-key-files");
    fs::create_dir_all(&no_files).unwrap();
    let keys =
        c.h.run("sign.keys", json!({ "ssh_dir": no_files, "agent": false }));
    let listed = |print: &str| {
        keys.as_array()
            .unwrap()
            .iter()
            .find(|key| key["fingerprint"] == print)
            .unwrap_or_else(|| panic!("{print} is not listed: {keys}"))
            .clone()
    };
    assert_eq!(listed(&leaf_print)["usable"], true);
    assert_eq!(listed(&leaf_print)["source"], "gpgsm");
    assert_eq!(listed(&old_print)["usable"], false);

    // Three commits: the chain carried in the signature (gpgsm's way, all
    // but the root), the signer's certificate alone, and one made in 2020
    // with the certificate that has expired since.
    let carried = c.commit("chain-carried.txt");
    // And a tag signed the same way, which `tag.details` judges.
    c.h.run(
        "tag.create",
        json!({ "name": "certified", "message": "signed with a certificate", "sign": true }),
    );
    let alone_signer = c.wrapper("signer-alone", &signer_home, "--include-certs 1");
    c.h.set("gpg.x509.program", alone_signer.to_str().unwrap());
    let alone = c.commit("leaf-alone.txt");
    let in_2020 = c.wrapper(
        "signer-2020",
        &signer_home,
        "--faked-system-time 20200601T000000",
    );
    c.h.set("gpg.x509.program", in_2020.to_str().unwrap());
    c.h.set("user.signingkey", &old_print);
    let expired = c.commit("expired.txt");
    c.h.set("user.signingkey", &leaf_print);

    let signer_name = |verdict: &Value| {
        let signer = verdict["signer"].as_str().unwrap_or_default().to_owned();
        assert!(signer.contains("CN=Ada Fixture"), "{verdict}");
    };
    let certificates: [&Path; 2] = [&leaf, &old];

    // The chain trusted at its root, the intermediate in the store.
    let good = c.judged(
        &Store {
            name: "trusted",
            intermediate: true,
            trusted: true,
            lists: Lists::Off,
        },
        &certificates,
        &carried,
    );
    assert_eq!(good["state"], "good", "{good}");
    assert_eq!(good["fingerprint"], leaf_print.as_str());
    assert_eq!(good["trust"], "fully");
    assert!(good["chain"].is_null());
    signer_name(&good);
    let tag = c.h.tag_details("certified");
    assert_eq!(tag["signature"]["state"], "good", "{tag}");
    assert!(tag["signature"]["chain"].is_null(), "{tag}");

    // The intermediate only in the signature, which is enough.
    let good = c.judged(
        &Store {
            name: "carried",
            intermediate: false,
            trusted: true,
            lists: Lists::Off,
        },
        &certificates,
        &carried,
    );
    assert_eq!(good["state"], "good", "{good}");

    // The intermediate nowhere: the signature carries the signer's
    // certificate alone, and the store has none between it and the root.
    let missing_issuer = c.judged(
        &Store {
            name: "no-intermediate",
            intermediate: false,
            trusted: true,
            lists: Lists::Off,
        },
        &certificates,
        &alone,
    );
    assert_eq!(missing_issuer["state"], "untrusted", "{missing_issuer}");
    assert_eq!(missing_issuer["chain"], "missing_issuer");
    assert_eq!(missing_issuer["trust"], "undefined");
    signer_name(&missing_issuer);

    // The root not trusted.
    let untrusted = c.judged(
        &Store {
            name: "untrusted-root",
            intermediate: true,
            trusted: false,
            lists: Lists::Off,
        },
        &certificates,
        &carried,
    );
    assert_eq!(untrusted["state"], "untrusted", "{untrusted}");
    assert_eq!(untrusted["chain"], "untrusted_root");
    c.tag_judged("untrusted_root");

    // The certificate expired since it signed.
    let old_one = c.judged(
        &Store {
            name: "expired",
            intermediate: true,
            trusted: true,
            lists: Lists::Off,
        },
        &certificates,
        &expired,
    );
    assert_eq!(old_one["state"], "key_expired", "{old_one}");
    assert_eq!(old_one["fingerprint"], old_print.as_str());
    assert!(old_one["chain"].is_null());

    // Revocation lists: the root's (which revokes nothing) first, since
    // dirmngr checks the issuer of a list before it takes it, then the
    // intermediate's.
    let Some(root_list) = c
        .authority
        .revocation_list(&c.authority.root.clone(), "root")
    else {
        return;
    };
    let intermediate_dir = c.authority.intermediate.clone();
    let Some(clean_list) = c.authority.revocation_list(&intermediate_dir, "clean") else {
        return;
    };
    if !c.authority.revoke(&leaf) {
        return;
    }
    let Some(revoking_list) = c.authority.revocation_list(&intermediate_dir, "revoking") else {
        return;
    };

    // Lists that revoke nothing: good, with the lists checked.
    let good = c.judged(
        &Store {
            name: "lists-clean",
            intermediate: true,
            trusted: true,
            lists: Lists::Loaded(&[&root_list, &clean_list]),
        },
        &certificates,
        &carried,
    );
    assert_eq!(good["state"], "good", "{good}");

    // The signer's certificate revoked.
    let revoked = c.judged(
        &Store {
            name: "revoked",
            intermediate: true,
            trusted: true,
            lists: Lists::Loaded(&[&root_list, &revoking_list]),
        },
        &certificates,
        &carried,
    );
    assert_eq!(revoked["state"], "untrusted", "{revoked}");
    assert_eq!(revoked["chain"], "revoked");
    assert_eq!(revoked["trust"], "never");
    signer_name(&revoked);
    c.tag_judged("revoked");

    // No list to be had: the distribution point is not to be reached.
    let unknown = c.judged(
        &Store {
            name: "no-lists",
            intermediate: true,
            trusted: true,
            lists: Lists::Loaded(&[]),
        },
        &certificates,
        &carried,
    );
    assert_eq!(unknown["state"], "untrusted", "{unknown}");
    assert_eq!(unknown["chain"], "revocation_unknown");
    assert!(
        unknown["reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );
}
