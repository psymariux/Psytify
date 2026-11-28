# R8 rules of the release build.

# JNA finds its own classes, structures and callbacks by name from native code.
-keep class com.sun.jna.** { *; }
-keepclassmembers class * extends com.sun.jna.** { public *; }
# JNA's desktop helpers name AWT, which Android does not have.
-dontwarn java.awt.**

# The UniFFI bindings (generated): their JNA structures are read field by field through
# reflection and their callback interfaces are called from Rust.
-keep class dev.psytify.app.rust.** { *; }
