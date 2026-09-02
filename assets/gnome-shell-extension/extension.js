import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import St from 'gi://St';

import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';

const RUNTIME_DIRECTORY = 'peasy-user';
const UI = '/run/current-system/sw/bin/peasy-ui';

const PeasyLauncher = GObject.registerClass(class PeasyLauncher extends PanelMenu.Button {
    _init() {
        // Peasy is a launcher, so do not create PanelMenu.Button's popup menu
        // or enable its menu-opening click gesture.
        super._init(0.0, 'Peasy', true);

        const dot = new St.Widget({
            style_class: 'peasy-launcher-dot',
            accessible_name: 'Open Peasy',
            x_expand: false,
            y_expand: false,
        });
        this.add_child(new St.Bin({
            child: dot,
            x_align: Clutter.ActorAlign.CENTER,
            y_align: Clutter.ActorAlign.CENTER,
        }));

        // GNOME Shell 50 panel buttons use ClickGesture rather than the legacy
        // actor button signals. Recognizing on press also matches the native
        // PanelMenu.Button behaviour.
        this._launchGesture = new Clutter.ClickGesture();
        this._launchGesture.set_recognize_on_press(true);
        this._launchGesture.connect('recognize', () => {
            this._open();
        });
        this.add_action(this._launchGesture);

        this.connect('key-press-event', (_actor, event) => {
            const key = event.get_key_symbol();
            if (key !== Clutter.KEY_Return &&
                key !== Clutter.KEY_KP_Enter &&
                key !== Clutter.KEY_space)
                return Clutter.EVENT_PROPAGATE;

            this._open();
            return Clutter.EVENT_STOP;
        });

        this._runtime = GLib.build_filenamev([
            GLib.get_user_runtime_dir(),
            RUNTIME_DIRECTORY,
        ]);
        GLib.mkdir_with_parents(this._runtime, 0o700);
        this._readyPath = GLib.build_filenamev([this._runtime, 'panel-ready']);
        GLib.file_set_contents(this._readyPath, 'ready');
        GLib.chmod(this._readyPath, 0o600);
    }

    _open() {
        try {
            Gio.Subprocess.new([UI], Gio.SubprocessFlags.NONE);
        } catch (error) {
            Main.notifyError('Could not open Peasy', error.message);
        }
    }

    destroy() {
        GLib.unlink(this._readyPath);
        super.destroy();
    }
});

export default class PeasyExtension extends Extension {
    enable() {
        this._indicator = new PeasyLauncher();
        Main.panel.addToStatusArea('peasy', this._indicator, 0, 'right');
    }

    disable() {
        this._indicator?.destroy();
        this._indicator = null;
    }
}
