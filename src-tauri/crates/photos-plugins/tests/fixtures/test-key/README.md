# TEST ONLY — never trust this key

An unencrypted minisign key pair for `photos-plugins` tests and D1's `pack.test.mjs`. The secret
key is public on purpose. No Lumen build may ever ship it as the JL Studios key or accept it
outside tests. Regenerate with `cargo test -p photos-plugins make_test_key -- --ignored`
(then re-sign any fixture that uses it).
