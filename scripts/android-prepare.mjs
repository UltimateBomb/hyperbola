// Patches the Android project that `tauri android init` generates.
//
// The generated project is not committed — it is regenerated on every build —
// so the few Android-specific things Hyperbola needs are applied here instead
// of being hand-edited into a file that would be overwritten anyway.
//
// Two things must be true for the download engine to work:
//   * native libraries must stay extracted on disk, because the bundled
//     Python is executed as a real file, not mapped out of the APK;
//   * every ABI youtubedl-android ships must survive packaging.
import { readFileSync, writeFileSync, existsSync } from "node:fs";

const manifestPath = "src-tauri/gen/android/app/src/main/AndroidManifest.xml";
const gradlePath = "src-tauri/gen/android/app/build.gradle.kts";

const permissions = [
  "android.permission.INTERNET",
  "android.permission.ACCESS_NETWORK_STATE",
  "android.permission.FOREGROUND_SERVICE",
  "android.permission.FOREGROUND_SERVICE_DATA_SYNC",
  "android.permission.POST_NOTIFICATIONS",
  "android.permission.WAKE_LOCK",
  // Sideloaded updates: the app hands the downloaded APK to the system installer.
  "android.permission.REQUEST_INSTALL_PACKAGES",
];

function patchManifest() {
  if (!existsSync(manifestPath)) throw new Error(`missing ${manifestPath} — run "tauri android init" first`);
  let xml = readFileSync(manifestPath, "utf8");

  const missing = permissions.filter((name) => !xml.includes(`"${name}"`));
  if (missing.length > 0) {
    const lines = missing.map((name) => `    <uses-permission android:name="${name}" />`).join("\n");
    xml = xml.replace("<application", `${lines}\n\n    <application`);
  }
  // Legacy storage permission, needed only up to Android 9.
  if (!xml.includes("WRITE_EXTERNAL_STORAGE")) {
    xml = xml.replace(
      "<application",
      '    <uses-permission android:name="android.permission.WRITE_EXTERNAL_STORAGE" android:maxSdkVersion="28" />\n\n    <application',
    );
  }
  if (!xml.includes("android:extractNativeLibs")) {
    xml = xml.replace("<application", '<application\n        android:extractNativeLibs="true"');
  }
  writeFileSync(manifestPath, xml);
  console.log(`patched ${manifestPath}`);
}

function patchGradle() {
  if (!existsSync(gradlePath)) throw new Error(`missing ${gradlePath}`);
  let gradle = readFileSync(gradlePath, "utf8");
  // The download engine unpacks its own Python and ffmpeg through reflection,
  // and R8 renames the classes it needs: the release build installed fine and
  // then died on first launch inside YoutubeDL.init. Shrinking the dex saved
  // around 8 MB in an app whose weight is native libraries, which is not a
  // trade worth a crash.
  if (gradle.includes("isMinifyEnabled = true")) {
    gradle = gradle.replace(/isMinifyEnabled = true/g, "isMinifyEnabled = false");
  }
  if (!gradle.includes("useLegacyPackaging")) {
    gradle = gradle.replace(
      /android\s*\{/,
      `android {
    packaging {
        jniLibs {
            // The engine executes its Python from disk, so the libraries
            // cannot stay compressed inside the APK.
            useLegacyPackaging = true
        }
    }`,
    );
  }
  writeFileSync(gradlePath, gradle);
  console.log(`patched ${gradlePath}`);
}

patchManifest();
patchGradle();
