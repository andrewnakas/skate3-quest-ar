//! Requests the two Scene API permissions at runtime.
//!
//! Both are already declared in `AndroidManifest.xml` (see
//! `Cargo.toml`'s `[package.metadata.android]` block), but Quest/Android
//! treat `com.oculus.permission.USE_SCENE` and `USE_ANCHOR_API` as
//! dangerous permissions: without an explicit `requestPermissions` call the
//! OpenXR runtime silently returns `XR_ERROR_PLANE_DETECTION_PERMISSION_DENIED_EXT`
//! forever, even though the manifest lists them. There is no Activity
//! subclass in this NativeActivity app to put a Java-side call in, so this
//! reaches `Activity.requestPermissions` directly over JNI.
//!
//! This needs the *Activity* object specifically — `requestPermissions` isn't
//! defined on `Application` (which is what android-activity puts in
//! `ndk_context` until `activity_context` swaps it; this module doesn't rely
//! on that swap).
//! `bevy::android::ANDROID_APP` is the `AndroidApp` bevy's `#[bevy_main]`
//! stashes before calling into this crate, and its `activity_as_ptr()` is
//! the real `NativeActivity` instance.
use jni::objects::{JObject, JObjectArray, JString, JValue};
use jni::{jni_sig, jni_str, JavaVM};

const PERMISSIONS: [&str; 2] = [
    "com.oculus.permission.USE_SCENE",
    "com.oculus.permission.USE_ANCHOR_API",
];

/// `PackageManager.PERMISSION_GRANTED`.
const PERMISSION_GRANTED: i32 = 0;

/// Fire-and-forget: shows the OS consent dialog for any permission that
/// isn't granted yet. The grant lands asynchronously (there's no
/// `onRequestPermissionsResult` handler here), so callers just keep retrying
/// the OpenXR call until it stops returning a permission error.
///
/// Already-granted permissions are skipped and, when nothing is missing, no
/// request is made at all: `requestPermissions` always starts
/// GrantPermissionsActivity (a Quest "exclusive system dialog"), which pauses
/// and stops this activity mid-startup even when it has nothing to ask.
pub fn request_scene_permissions() {
    let Some(app) = bevy::android::ANDROID_APP.get() else {
        eprintln!("skate_quest: ANDROID_APP not set yet, skipping permission request");
        return;
    };
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<Vec<&'static str>> {
        let activity = unsafe { JObject::from_raw(&*env, app.activity_as_ptr().cast()) };
        let mut missing = Vec::new();
        for name in PERMISSIONS {
            let jname = JString::from_str(env, name)?;
            let state = env
                .call_method(
                    &activity,
                    jni_str!("checkSelfPermission"),
                    jni_sig!("(Ljava/lang/String;)I"),
                    &[JValue::from(&jname)],
                )?
                .i()?;
            if state != PERMISSION_GRANTED {
                missing.push(name);
            }
        }
        let Some(first) = missing.first() else {
            return Ok(missing);
        };
        let first = JString::from_str(env, first)?;
        let array = JObjectArray::<JString>::new(env, missing.len(), &first)?;
        for (i, name) in missing.iter().enumerate().skip(1) {
            let jname = JString::from_str(env, name)?;
            array.set_element(env, i, &jname)?;
        }
        env.call_method(
            &activity,
            jni_str!("requestPermissions"),
            jni_sig!("([Ljava/lang/String;I)V"),
            &[JValue::from(&array), JValue::from(0i32)],
        )?;
        Ok(missing)
    });
    match result {
        Ok(missing) if missing.is_empty() => {
            eprintln!("skate_quest: {PERMISSIONS:?} already granted")
        }
        Ok(missing) => eprintln!("skate_quest: requested {missing:?}"),
        Err(e) => eprintln!("skate_quest: requestPermissions failed: {e}"),
    }
}
