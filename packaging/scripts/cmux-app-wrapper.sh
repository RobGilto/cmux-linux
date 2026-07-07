#!/bin/sh
# cmux-app launcher — auto-detects display backend for GTK4 GL compatibility.
#
# GTK4 may choose Wayland/EGL even in X11 sessions if Wayland libraries are
# present, causing GL context creation failures on NVIDIA proprietary drivers.
# This wrapper forces X11/GLX when running under an X11 session.

if [ -z "$GDK_BACKEND" ]; then
    case "${XDG_SESSION_TYPE}" in
        x11)
            export GDK_BACKEND=x11
            ;;
        wayland)
            # Check for NVIDIA proprietary driver — EGL often fails
            if command -v nvidia-smi >/dev/null 2>&1; then
                export GDK_BACKEND=x11
            fi
            ;;
    esac
fi

# GDK binds the GLES API at EGL init by default and then cannot hand out the
# desktop OpenGL context libghostty's renderer requires, so GLArea realize
# fails with "Unable to create a GL context" on NVIDIA proprietary drivers
# (regardless of backend). gl-prefer-gl makes GDK bind desktop GL up front.
if command -v nvidia-smi >/dev/null 2>&1; then
    export GDK_DEBUG="${GDK_DEBUG:+${GDK_DEBUG},}gl-prefer-gl"
fi

exec /usr/bin/cmux-app.bin "$@"
