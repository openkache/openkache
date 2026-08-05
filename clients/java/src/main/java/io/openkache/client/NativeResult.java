package io.openkache.client;

import com.sun.jna.Pointer;

record NativeResult(int kind, byte[] payload, Pointer client) {}
