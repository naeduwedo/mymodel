// Copyright (c) GGUF AI Engine contributors. Apache-2.0.

using Microsoft.UI.Xaml;

namespace GgufAiEngine.Shell;

/// <summary>
/// Application entry point. Everything it does is start a window; the shell
/// contains no engine logic (spec §2.1: "UI is not the owner of inference").
/// </summary>
public partial class App : Application
{
    private Window? _window;

    public App()
    {
        InitializeComponent();
    }

    protected override void OnLaunched(LaunchActivatedEventArgs args)
    {
        var window = new MainWindow();
        _window = window;
        // Stop polling and release the channel on close; a background poll loop
        // outliving the window would keep the app alive after the user closed it.
        window.Closed += (_, _) => window.Shutdown();
        window.Activate();
    }
}