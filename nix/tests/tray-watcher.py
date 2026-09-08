"""Test-only watcher: hold the bus name but reject the first registration."""

from gi.repository import Gio, GLib

XML = """<node><interface name="org.kde.StatusNotifierWatcher">
<method name="RegisterStatusNotifierItem"><arg type="s" direction="in"/></method>
<property name="RegisteredStatusNotifierItems" type="as" access="read"/>
<property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
<property name="ProtocolVersion" type="i" access="read"/>
</interface></node>"""

registrations = []
rejected = False


def method_call(connection, sender, path, interface, method, parameters, invocation):
    global rejected
    if not rejected:
        rejected = True
        print("registration rejected", flush=True)
        invocation.return_dbus_error(
            "org.kde.StatusNotifierWatcher.NotReady", "Tray host is still starting"
        )
        return
    registrations.append(parameters.unpack()[0])
    invocation.return_value(None)
    print("registration accepted", flush=True)


def get_property(connection, sender, path, interface, name):
    return {
        "RegisteredStatusNotifierItems": GLib.Variant("as", registrations),
        "IsStatusNotifierHostRegistered": GLib.Variant("b", True),
        "ProtocolVersion": GLib.Variant("i", 0),
    }[name]


def bus_acquired(connection, name):
    connection.register_object(
        "/StatusNotifierWatcher",
        Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0],
        method_call,
        get_property,
        None,
    )


Gio.bus_own_name(
    Gio.BusType.SESSION,
    "org.kde.StatusNotifierWatcher",
    Gio.BusNameOwnerFlags.NONE,
    bus_acquired,
    None,
    None,
)
GLib.MainLoop().run()
