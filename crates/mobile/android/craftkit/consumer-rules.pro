# The Rust library calls these by name over JNI.
-keep class dev.craft.kit.CraftNative { *; }
-keep interface dev.craft.kit.CraftNative$Handler { *; }
-keepclassmembers class * implements dev.craft.kit.CraftNative$Handler { void onRequest(int, java.lang.String, byte[]); }
