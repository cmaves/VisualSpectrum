#!/bin/sh

if [ "$1" = "d" ] || [ "$1" = "debug" ]; then
    cargo build 
else 
    cargo build --release
fi

if [ "$?" != 0 ]; then
    exit 0
fi

sudo echo get sudo 
sleep 3 && sudo jack_connect system:midi_capture_1 spectrum:midi_in&

if [ "$1" = "d" ] || [ "$1" = "debug" ]; then
    sudo target/debug/spectrum
else
    sudo target/release/spectrum
fi
