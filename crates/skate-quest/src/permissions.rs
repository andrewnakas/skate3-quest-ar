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

/// Fire-and-forget: shows the OS consent dialog if it hasn't been answered
/// yet. The grant lands asynchronously (there's no `onRequestPermissionsResult`
/// handler here), so callers just keep retrying the OpenXR call until it
/// stops returning a permission error.
pub fn request_scene_permissions() {
    let Some(app) = bevy::android::ANDROID_APP.get() else {
        eprintln!("skate_quest: ANDROID_APP not set yet, skipping permission request");
        return;
    };
    let vm = unsafe { JavaVM::from_raw(app.vm_as_ptr().cast()) };
    let result = vm.attach_current_thread(|env| -> jni::errors::Result<()> {
        let activity = unsafe { JObject::from_raw(&*env, app.activity_as_ptr().cast()) };
        let first = JString::from_str(env, PERMISSIONS[0])?;
        let array = JObjectArray::<JString>::new(env, PERMISSIONS.len(), &first)?;
        for (i, name) in PERMISSIONS.iter().enumerate().skip(1) {
            let jname = JString::from_str(env, name)?;
            array.set_element(env, i, &jname)?;
        }
        env.call_method(
            &activity,
            jni_str!("requestPermissions"),
            jni_sig!("([Ljava/lang/String;I)V"),
            &[JValue::from(&array), JValue::from(0i32)],
        )?;
        Ok(())
    });
    match result {
        Ok(()) => eprintln!("skate_quest: requested {PERMISSIONS:?}"),
        Err(e) => eprintln!("skate_quest: requestPermissions failed: {e}"),
    }
}
