// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System.Linq;
using Gguf.V1;
using GgufAiEngine.Shell.Core;
using Xunit;

namespace GgufAiEngine.Shell.Core.Tests;

/// <summary>
/// Endpoint derivation must match the Rust core exactly; the vectors here are
/// duplicated in <c>crates/protocol/src/features.rs</c>.
/// </summary>
public class EndpointTests
{
    [Theory]
    [InlineData("C:/data", "cc34379d6c404275")]
    [InlineData("F:/dsh-project/myModel/data", "046164204fac7302")]
    [InlineData("/home/user/.local/share/gguf-ai-engine", "8d31b980b94baba7")]
    public void Tag_matches_the_rust_implementation(string dataRoot, string expected) =>
        Assert.Equal(expected, Endpoint.Tag(dataRoot));

    [Fact]
    public void Tag_is_case_insensitive_like_windows_paths() =>
        Assert.Equal(Endpoint.Tag("C:/data"), Endpoint.Tag("c:/DATA"));

    [Fact]
    public void Windows_endpoint_is_a_local_pipe()
    {
        var endpoint = Endpoint.For("C:/data");
        Assert.Equal("\\\\.\\pipe\\gguf-ai-engine-cc34379d6c404275", endpoint);
    }

    [Fact]
    public void Unix_endpoint_is_a_socket_under_the_run_directory() =>
        Assert.Equal(
            System.IO.Path.Combine("/data", "run", "core.sock"),
            Endpoint.For("/data", windows: false));

    [Fact]
    public void Different_data_roots_never_share_an_endpoint() =>
        Assert.NotEqual(Endpoint.For("C:/a"), Endpoint.For("C:/b"));
}

/// <summary>
/// The mapper is the only logic the shell owns, so it is the only thing worth
/// testing headlessly. These tests use hand-built snapshots: the shell must
/// render exactly what the core sends, including gaps.
/// </summary>
public class SnapshotMapperTests
{
    private static UiSnapshot SampleSnapshot()
    {
        var snapshot = new UiSnapshot
        {
            SnapshotVersion = 1,
            GeneratedAtLocal = "2026-01-01 00:00:00 UTC",
            Overview = new UiOverview { AttentionRequired = true },
            Settings = new UiSettings
            {
                ConfigFile = "C:/data/config.toml",
                ConfigFileExists = false,
            },
        };

        snapshot.Overview.Tiles.Add(new UiTile
        {
            Id = "core.state",
            Label = "Core state",
            Value = "ready",
            Severity = UiSeverity.Ok,
        });
        snapshot.Overview.Tiles.Add(new UiTile
        {
            Id = "features.missing",
            Label = "Not implemented",
            Value = "8 capabilities",
            Detail = "Hardware inventory, llama.cpp runtime",
            Severity = UiSeverity.Disabled,
        });
        snapshot.Overview.Features.Add(new UiFeatureRow
        {
            Id = "ipc",
            Label = "IPC transport",
            StatusText = "supported",
            Severity = UiSeverity.Ok,
        });
        snapshot.Overview.Features.Add(new UiFeatureRow
        {
            Id = "runtime_llama_cpp",
            Label = "llama.cpp runtime",
            StatusText = "planned",
            Reason = "Task 6: no runtime adapter is installed",
            Severity = UiSeverity.Disabled,
        });
        snapshot.Overview.RecentEvents.Add(new UiEventLine
        {
            Seq = 1,
            AtLocal = "00:00:01 UTC",
            Text = "Core state starting -> ready",
            Severity = UiSeverity.Ok,
        });

        var group = new UiSettingsGroup { Title = "Core" };
        group.Rows.Add(new UiSettingRow
        {
            Key = "core.api_port",
            Label = "API port",
            Value = "18888",
            Note = "Read-only in this build",
            Editable = false,
            Severity = UiSeverity.Ok,
        });
        snapshot.Settings.Groups.Add(group);

        return snapshot;
    }

    [Fact]
    public void Tiles_keep_the_order_and_values_the_core_sent()
    {
        var view = SnapshotMapper.ToOverview(SampleSnapshot());
        Assert.Equal(new[] { "core.state", "features.missing" }, view.Tiles.Select(t => t.Id));
        Assert.Equal("ready", view.Tiles[0].Value);
        Assert.False(view.Tiles[0].HasDetail);
        Assert.True(view.Tiles[1].HasDetail);
        Assert.Contains("llama.cpp", view.Tiles[1].Detail);
    }

    [Fact]
    public void Missing_capabilities_keep_their_reason()
    {
        var view = SnapshotMapper.ToOverview(SampleSnapshot());
        var planned = view.Features.Single(f => f.Id == "runtime_llama_cpp");
        Assert.Equal("planned", planned.StatusText);
        Assert.False(string.IsNullOrWhiteSpace(planned.Reason));
    }

    [Fact]
    public void Settings_are_reported_as_read_only() 
    {
        var view = SnapshotMapper.ToSettings(SampleSnapshot());
        var row = view.Groups.Single().Rows.Single();
        Assert.False(row.Editable);
        Assert.False(string.IsNullOrWhiteSpace(row.Note));
        Assert.Contains("defaults in use", view.SourceLine);
    }

    [Fact]
    public void Events_are_rendered_as_time_then_text()
    {
        var view = SnapshotMapper.ToOverview(SampleSnapshot());
        Assert.Single(view.RecentEvents);
        Assert.StartsWith("00:00:01 UTC", view.RecentEvents[0]);
    }

    [Fact]
    public void An_empty_snapshot_does_not_throw()
    {
        var view = SnapshotMapper.ToOverview(new UiSnapshot());
        Assert.Empty(view.Tiles);
        Assert.Empty(view.Features);
        Assert.Empty(view.RecentEvents);
        Assert.False(view.AttentionRequired);
    }

    [Theory]
    [InlineData(UiSeverity.Ok, false)]
    [InlineData(UiSeverity.Disabled, false)]
    [InlineData(UiSeverity.Warning, true)]
    [InlineData(UiSeverity.Error, true)]
    public void Attention_is_decided_by_severity(UiSeverity severity, bool expected) =>
        Assert.Equal(expected, SnapshotMapper.NeedsAttention(severity));

    [Fact]
    public void Every_severity_has_a_distinct_glyph()
    {
        var glyphs = new[]
        {
            SnapshotMapper.GlyphFor(UiSeverity.Ok),
            SnapshotMapper.GlyphFor(UiSeverity.Warning),
            SnapshotMapper.GlyphFor(UiSeverity.Error),
            SnapshotMapper.GlyphFor(UiSeverity.Disabled),
        };
        Assert.Equal(glyphs.Length, glyphs.Distinct().Count());
        Assert.All(glyphs, g => Assert.False(string.IsNullOrWhiteSpace(g)));
    }
}
