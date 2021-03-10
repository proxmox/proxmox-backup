Ext.define('pbs-datastore-list', {
    extend: 'Ext.data.Model',
    fields: ['name', 'comment'],
    proxy: {
        type: 'proxmox',
        url: "/api2/json/admin/datastore",
    },
    idProperty: 'store',
});

Ext.define('pbs-tape-drive-list', {
    extend: 'Ext.data.Model',
    fields: ['name', 'changer'],
    proxy: {
        type: 'proxmox',
        url: "/api2/json/tape/drive",
    },
    idProperty: 'name',
});

Ext.define('PBS.store.NavigationStore', {
    extend: 'Ext.data.TreeStore',

    storeId: 'NavigationStore',

    root: {
	expanded: true,
	children: [
	    {
		text: gettext('Dashboard'),
		iconCls: 'fa fa-tachometer',
		path: 'pbsDashboard',
		leaf: true,
	    },
	    {
		text: gettext('Configuration'),
		iconCls: 'fa fa-gears',
		path: 'pbsSystemConfiguration',
		expanded: true,
		children: [
		    {
			text: gettext('Access Control'),
			iconCls: 'fa fa-key',
			path: 'pbsAccessControlPanel',
			leaf: true,
		    },
		    {
			text: gettext('Remotes'),
			iconCls: 'fa fa-server',
			path: 'pbsRemoteView',
			leaf: true,
		    },
		    {
			text: gettext('Subscription'),
			iconCls: 'fa fa-support',
			path: 'pbsSubscription',
			leaf: true,
		    },
		],
	    },
	    {
		text: gettext('Administration'),
		iconCls: 'fa fa-wrench',
		path: 'pbsServerAdministration',
		expanded: true,
		leaf: false,
		children: [
		    {
			text: gettext('Shell'),
			iconCls: 'fa fa-terminal',
			path: 'pbsXtermJsConsole',
			leaf: true,
		    },
		    {
			text: gettext('Storage / Disks'),
			iconCls: 'fa fa-hdd-o',
			path: 'pbsStorageAndDiskPanel',
			leaf: true,
		    },
		],
	    },
	    {
		text: gettext('Datastore'),
		iconCls: 'fa fa-archive',
		id: 'datastores',
		path: 'pbsDataStores',
		expanded: true,
		expandable: false,
		leaf: false,
		children: [
		    {
			text: gettext('Add Datastore'),
			iconCls: 'fa fa-plus-circle',
			leaf: true,
			id: 'addbutton',
		    },
		],
	    },
	],
    },
});

Ext.define('PBS.view.main.NavigationTree', {
    extend: 'Ext.list.Tree',
    xtype: 'navigationtree',

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    view.rstore = Ext.create('Proxmox.data.UpdateStore', {
		autoStart: true,
		interval: 15 * 1000,
		storeid: 'pbs-datastore-list',
		model: 'pbs-datastore-list',
	    });

	    view.rstore.on('load', this.onLoad, this);
	    view.on('destroy', view.rstore.stopUpdate);

	    if (PBS.enableTapeUI) {
		if (view.tapestore === undefined) {
		    view.tapestore = Ext.create('Proxmox.data.UpdateStore', {
			autoStart: true,
			interval: 60 * 1000,
			storeid: 'pbs-tape-drive-list',
			model: 'pbs-tape-drive-list',
		    });
		    view.tapestore.on('load', this.onTapeDriveLoad, this);
		    view.on('destroy', view.tapestore.stopUpdate);
		}

		let root = view.getStore().getRoot();
		if (root.findChild('id', 'tape_management', false) === null) {
		    root.insertChild(3, {
			text: "Tape Backup",
			iconCls: 'pbs-icon-tape',
			id: 'tape_management',
			path: 'pbsTapeManagement',
			expanded: true,
			children: [],
		    });
		}
	    }
	},

	onTapeDriveLoad: function(store, records, success) {
	    if (!success) return;

	    let view = this.getView();
	    let root = view.getStore().getRoot();

	    records.sort((a, b) => a.data.name.localeCompare(b.data.name));
	    let list = root.findChild('id', 'tape_management', false);
	    let newSet = {};

	    for (const drive of records) {
		let path, text, iconCls;
		if (drive.data.changer !== undefined) {
		    text = drive.data.changer;
		    path = `Changer-${text}`;
		    iconCls = 'fa fa-exchange';
		} else {
		    text = drive.data.name;
		    path = `Drive-${text}`;
		    iconCls = 'pbs-icon-tape-drive';
		}
		newSet[path] = {
		    text,
		    path,
		    iconCls,
		    leaf: true,
		};
	    }

	    let paths = Object.keys(newSet).sort();

	    let oldIdx = 0;
	    for (let newIdx = 0; newIdx < paths.length; newIdx++) {
		let newPath = paths[newIdx];
		// find index to insert
		while (oldIdx < list.childNodes.length && newPath > list.getChildAt(oldIdx).data.path) {
		    oldIdx++;
		}

		if (oldIdx >= list.childNodes.length || list.getChildAt(oldIdx).data.path !== newPath) {
		    list.insertChild(oldIdx, newSet[newPath]);
		}
	    }

	    let toremove = [];
	    list.eachChild((child) => {
		if (!newSet[child.data.path]) {
		    toremove.push(child);
		}
	    });
	    toremove.forEach((child) => list.removeChild(child, true));

	    if (view.pathToSelect !== undefined) {
		let path = view.pathToSelect;
		delete view.pathToSelect;
		view.select(path, true);
	    }
	},

	onLoad: function(store, records, success) {
	    if (!success) return;
	    var view = this.getView();

	    let root = view.getStore().getRoot();

	    records.sort((a, b) => a.id.localeCompare(b.id));

	    var list = root.findChild('id', 'datastores', false);
	    var length = records.length;
	    var lookup_hash = {};
	    let j = 0;
	    for (let i = 0; i < length; i++) {
		let name = records[i].id;
		lookup_hash[name] = true;

		while (name.localeCompare(list.getChildAt(j).data.text) > 0 &&
		       (j + 1) < list.childNodes.length) {
		    j++;
		}

		if (list.getChildAt(j).data.text.localeCompare(name) !== 0) {
		    list.insertChild(j, {
			text: name,
			path: `DataStore-${name}`,
			iconCls: 'fa fa-database',
			leaf: true,
		    });
		}
	    }

	    var erase_list = [];
	    list.eachChild(function(node) {
		let name = node.data.text;
		if (!lookup_hash[name] && node.data.id !== 'addbutton') {
		    erase_list.push(node);
		}
	    });

	    Ext.Array.forEach(erase_list, function(node) { list.removeChild(node, true); });

	    if (view.pathToSelect !== undefined) {
		let path = view.pathToSelect;
		delete view.pathToSelect;
		view.select(path, true);
	    }
	},
    },

    listeners: {
	itemclick: function(tl, info) {
	    if (info.node.data.id === 'addbutton') {
		let me = this;
		Ext.create('PBS.DataStoreEdit', {
		    listeners: {
			destroy: function() {
			    me.rstore.reload();
			},
		    },
		}).show();
		return false;
	    }
	    return true;
	},
    },

    reloadTapeStore: function() {
	let me = this;
	if (!PBS.enableTapeUI) {
	    return;
	}

	me.tapestore.load();
    },

    select: function(path, silent) {
	var me = this;
	if (me.rstore.isLoaded() && (!PBS.enableTapeUI || me.tapestore.isLoaded())) {
	    if (silent) {
		me.suspendEvents(false);
	    }
	    var item = me.getStore().findRecord('path', path, 0, false, true, true);
	    me.setSelection(item);
	    if (silent) {
		me.resumeEvents(true);
	    }
	} else {
	    me.pathToSelect = path;
	}
    },

    animation: false,
    expanderOnly: true,
    expanderFirst: false,
    store: 'NavigationStore',
    ui: 'nav',
});
