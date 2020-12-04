Ext.define('pbs-datastore-list', {
    extend: 'Ext.data.Model',
    fields: ['name', 'comment'],
    proxy: {
        type: 'proxmox',
        url: "/api2/json/admin/datastore",
    },
    idProperty: 'store',
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
		storeId: 'pbs-datastore-list',
		storeid: 'pbs-datastore-list',
		model: 'pbs-datastore-list',
	    });

	    view.rstore.on('load', this.onLoad, this);
	    view.on('destroy', view.rstore.stopUpdate);
	},

	onLoad: function(store, records, success) {
	    if (!success) return;
	    var view = this.getView();

	    let root = view.getStore().getRoot();

	    if (PBS.TapeManagement !== undefined) {
		if (!root.findChild('id', 'tape_management', false)) {
		    root.insertChild(3, {
			text: "Tape Management",
			iconCls: 'fa fa-gears',
			id: 'tape_management',
			path: 'pbsTapeManagement',
			expanded: true,
			children: [],
		    });
		}
	    }

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

    select: function(path, silent) {
	var me = this;
	if (me.rstore.isLoaded()) {
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
