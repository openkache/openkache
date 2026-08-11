package io.openkache.client;

import com.sun.jna.Pointer;

record NativeResult(int kind, int status, byte[] payload, Pointer client) {}
