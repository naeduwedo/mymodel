// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System;
using System.IO;
using System.IO.Pipes;
using System.Threading;
using System.Threading.Tasks;
using Grpc.Core;
using Grpc.Net.Client;

namespace GgufAiEngine.Shell.Core;

/// <summary>
/// Derives IPC endpoints the same way the Rust core does.
/// </summary>
/// <remarks>
/// The two implementations must agree byte for byte, otherwise the shell looks
/// for a pipe that does not exist and every user sees "core not running". The
/// test vectors in <c>EndpointTests</c> are duplicated in
/// <c>crates/protocol/src/features.rs</c> to pin both sides.
/// </remarks>
public static class Endpoint
{
    /// <summary>Windows named-pipe prefix used by the core.</summary>
    public const string PipePrefix = @"\\.\pipe\";

    /// <summary>File name of the Unix socket under the data root.</summary>
    public const string SocketFileName = "core.sock";

    /// <summary>
    /// Endpoint for a data directory: a named pipe on Windows, a socket path
    /// elsewhere (the shells only run on Windows today, but the rule is shared).
    /// </summary>
    public static string For(string dataRoot, bool windows = true)
    {
        if (!windows)
        {
            return Path.Combine(dataRoot, "run", SocketFileName);
        }

        return PipePrefix + "gguf-ai-engine-" + Tag(dataRoot);
    }

    /// <summary>
    /// 64-bit FNV-1a of the lower-cased path, as 16 lowercase hex digits.
    /// </summary>
    public static string Tag(string dataRoot)
    {
        const ulong offsetBasis = 0xcbf2_9ce4_8422_2325UL;
        const ulong prime = 0x0000_0100_0000_01b3UL;

        var hash = offsetBasis;
        foreach (var b in System.Text.Encoding.UTF8.GetBytes(dataRoot.ToLowerInvariant()))
        {
            hash ^= b;
            hash *= prime;
        }

        return hash.ToString("x16");
    }
}
