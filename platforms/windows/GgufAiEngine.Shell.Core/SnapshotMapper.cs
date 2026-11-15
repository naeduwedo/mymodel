// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using System.Collections.Generic;
using System.Linq;
using Gguf.V1;

namespace GgufAiEngine.Shell.Core;

/// <summary>A single value shown on Overview.</summary>
public sealed record TileView(string Id, string Label, string Value, string Detail, UiSeverity Severity)
{
    /// <summary>Whether the detail line should be shown at all.</summary>
    public bool HasDetail => !string.IsNullOrWhiteSpace(Detail);
}

/// <summary>A capability row.</summary>
public sealed record FeatureView(string Id, string Label, string StatusText, string Reason, UiSeverity Severity);

/// <summary>A settings row.</summary>
public sealed record SettingRowView(string Key, string Label, string Value, string Note, bool Editable, UiSeverity Severity);

/// <summary>A titled group of settings rows.</summary>
public sealed record SettingGroupView(string Title, IReadOnlyList<SettingRowView> Rows);

/// <summary>Everything the window renders, derived from one snapshot.</summary>
public sealed record OverviewView(
    string GeneratedAt,
    bool AttentionRequired,
    IReadOnlyList<TileView> Tiles,
    IReadOnlyList<FeatureView> Features,
    IReadOnlyList<string> RecentEvents);

/// <summary>Settings page contents.</summary>
public sealed record SettingsView(
    string ConfigFile,
    bool ConfigFileExists,
    IReadOnlyList<SettingGroupView> Groups,
    IReadOnlyList<string> Issues)
{
    /// <summary>One-line description of where the configuration came from.</summary>
    public string SourceLine => ConfigFileExists
        ? $"Configuration: {ConfigFile}"
        : $"Configuration: {ConfigFile} (not found, defaults in use)";
}

/// <summary>
/// Maps the core's snapshot onto view models.
/// </summary>
/// <remarks>
/// This class contains no policy. Severity, labels, formatted values and the
/// reason a capability is unavailable all come from the core; a shell that
/// decided any of that itself would drift from the other two shells on the first
/// rename (spec §4).
/// </remarks>
public static class SnapshotMapper
{
    /// <summary>Overview view.</summary>
    public static OverviewView ToOverview(UiSnapshot snapshot)
    {
        var overview = snapshot.Overview;

        var tiles = (overview?.Tiles ?? Enumerable.Empty<UiTile>())
            .Select(t => new TileView(t.Id, t.Label, t.Value, t.Detail, t.Severity))
            .ToList();

        var features = (overview?.Features ?? Enumerable.Empty<UiFeatureRow>())
            .Select(f => new FeatureView(f.Id, f.Label, f.StatusText, f.Reason, f.Severity))
            .ToList();

        var events = (overview?.RecentEvents ?? Enumerable.Empty<UiEventLine>())
            .Select(e => $"{e.AtLocal}  {e.Text}")
            .ToList();

        return new OverviewView(
            snapshot.GeneratedAtLocal,
            overview?.AttentionRequired ?? false,
            tiles,
            features,
            events);
    }

    /// <summary>Settings view.</summary>
    public static SettingsView ToSettings(UiSnapshot snapshot)
    {
        var settings = snapshot.Settings;

        var groups = (settings?.Groups ?? Enumerable.Empty<UiSettingsGroup>())
            .Select(g => new SettingGroupView(
                g.Title,
                g.Rows.Select(r => new SettingRowView(r.Key, r.Label, r.Value, r.Note, r.Editable, r.Severity))
                    .ToList()))
            .ToList();

        var issues = (settings?.Issues ?? Enumerable.Empty<UiIssue>())
            .Select(i => $"{i.Field}: {i.Message}")
            .ToList();

        return new SettingsView(
            settings?.ConfigFile ?? string.Empty,
            settings?.ConfigFileExists ?? false,
            groups,
            issues);
    }

    /// <summary>
    /// Short glyph for a severity. Text rather than a colour so it survives
    /// high-contrast mode and colour blindness.
    /// </summary>
    public static string GlyphFor(UiSeverity severity) => severity switch
    {
        UiSeverity.Ok => "\u2713",
        UiSeverity.Warning => "!",
        UiSeverity.Error => "\u2715",
        UiSeverity.Disabled => "\u2013",
        _ => "?",
    };

    /// <summary>Whether a severity needs the user's attention.</summary>
    public static bool NeedsAttention(UiSeverity severity) =>
        severity is UiSeverity.Error or UiSeverity.Warning;
}
