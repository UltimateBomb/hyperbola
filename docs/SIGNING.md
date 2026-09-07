# The signing key

Android identifies an app by the key it was signed with. Change the key and
the phone treats the new build as a different app: it refuses to install over
the old one, and installing it fresh loses the queue, the settings and the
folder the user granted.

That key now exists once and does not change.

## Where it lives

- **The keystore itself:** `~/Documents/hyperbola-signing/hyperbola.p12` on the
  Mac, with its password next to it in `password.txt`. Not in this repository
  and never in it.
- **In CI:** repository secrets `ANDROID_KEYSTORE_BASE64` and
  `ANDROID_KEYSTORE_PASSWORD`. The release workflow decodes the keystore,
  signs every APK with it, and deletes it at the end of the run.

A build without that secret — someone else's fork — still produces an
installable APK, signed with a throwaway key generated for that run. Those
APKs cannot update each other, which is exactly what a throwaway key means.

## Keep it

If the keystore is lost, no future build can update an installed app: every
user has to uninstall and start over, losing their queue and settings. It
cannot be regenerated — a key with the same name is still a different key.

It is kept in four places, each verified byte for byte against the same
checksum:

| Machine | Path |
|---|---|
| the Mac that made it | `~/Documents/hyperbola-signing/` |
| the server | `~/secrets/hyperbola/` |
| the Windows machine | `D:\secrets\hyperbola\` |
| the external backup drive | `/Volumes/back/hyperbola-signing-key/` |

Each copy carries a plain note next to it explaining what the files are, for
whoever finds the folder without this document.

**The GitHub secret is not a backup.** A secret cannot be read back out; it
is the copy CI signs with, nothing more.

## Checking that a keystore is the right one

    openssl pkcs12 -in hyperbola.p12 -nokeys -passin file:password.txt \
      | openssl x509 -noout -fingerprint -sha256

It must print the fingerprint below. CI prints the same value while signing,
as `Signer #1 certificate SHA-256 digest`.

- keystore SHA-256: `0198e87eeb2b00b775468c64b1c2bd1e6d9a3f994cbad0222085a4ce3f3128a9`

## Facts about the current key

- 4096-bit RSA, SHA-256, valid until 2056
- fingerprint `7D:60:29:5F:AC:13:94:B3:CE:77:46:E2:10:A9:B2:32:F0:92:E8:29:C9:89:59:29:0B:AB:3F:83:7D:B6:F8:27`
