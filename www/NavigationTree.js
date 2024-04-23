Ext.define('pbs-datastore-list', {
    extend: 'Ext.data.Model',
    fields: ['name', 'comment', 'maintenance'],
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
		text: gettext('Notes'),
		iconCls: 'fa fa-sticky-note-o',
		path: 'pbsNodeNotes',
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
			text: gettext('Traffic Control'),
			iconCls: 'fa fa-signal fa-rotate-90',
			path: 'pbsTrafficControlView',
			leaf: true,
		    },
		    {
			text: gettext('Certificates'),
			iconCls: 'fa fa-certificate',
			path: 'pbsCertificateConfiguration',
			leaf: true,
		    },
		    {
			text: gettext('Notifications'),
			iconCls: 'fa fa-bell-o',
			path: 'pbsNotificationConfigView',
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
		text: "Tape Backup",
		iconCls: 'pbs-icon-tape',
		id: 'tape_management',
		path: 'pbsTapeManagement',
		expanded: true,
		children: [],
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
			virtualEntry: true,
		    },
		],
	    },
	],
    },
});

Ext.define('CustomTreeListItem', {
    extend: 'Ext.list.TreeItem',
    xtype: 'qtiptreelistitem',

    nodeUpdate: function(node, modifiedFieldNames) {
	this.callParent(arguments);
	const qtip = node ? node.get('qtip') : null;
	if (qtip) {
	    this.element.dom.setAttribute('data-qtip', qtip);
	} else {
	    this.element.dom.removeAttribute('data-qtip');
	}
    },
});

Ext.define('PBS.view.main.NavigationTree', {
    extend: 'Ext.list.Tree',
    xtype: 'navigationtree',

    animation: false,
    expanderOnly: true,
    expanderFirst: false,
    store: 'NavigationStore',
    ui: 'nav',

    defaults: {
	xtype: 'qtiptreelistitem',
    },

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    view.rstore = Ext.create('Proxmox.data.UpdateStore', {
		autoStart: true,
		interval: 15 * 1000,
		storeId: 'pbs-datastore-list', // NOTE: this is queried by selectors, avoid change!
		model: 'pbs-datastore-list',
	    });

	    view.rstore.on('load', this.onLoad, this);
	    view.on('destroy', view.rstore.stopUpdate);

	    if (view.tapeStore === undefined) {
		view.tapeStore = Ext.create('Proxmox.data.UpdateStore', {
		    autoStart: true,
		    interval: 60 * 1000,
		    storeid: 'pbs-tape-drive-list',
		    model: 'pbs-tape-drive-list',
		});
		view.tapeStore.on('load', this.onTapeDriveLoad, this);
		view.on('destroy', view.tapeStore.stopUpdate);
	    }
	},

	onTapeDriveLoad: function(store, records, success) {
	    if (!success) return;

	    let view = this.getView();
	    let root = view.getStore().getRoot();

	    records.sort((a, b) => a.data.name.localeCompare(b.data.name));

	    let list = root.findChild('id', 'tape_management', false);
	    let existingChildren = {};
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
		existingChildren[path] = {
		    text,
		    path,
		    iconCls,
		    leaf: true,
		};
	    }

	    let paths = Object.keys(existingChildren).sort();

	    let oldIdx = 0;
	    for (let newIdx = 0; newIdx < paths.length; newIdx++) {
		let newPath = paths[newIdx];
		// find index to insert
		while (oldIdx < list.childNodes.length && newPath > list.getChildAt(oldIdx).data.path) {
		    oldIdx++;
		}

		if (oldIdx >= list.childNodes.length || list.getChildAt(oldIdx).data.path !== newPath) {
		    list.insertChild(oldIdx, existingChildren[newPath]);
		}
	    }

	    let toRemove = [];
	    list.eachChild((child) => {
		if (!existingChildren[child.data.path]) {
		    toRemove.push(child);
		}
	    });
	    toRemove.forEach((child) => list.removeChild(child, true));

	    if (view.pathToSelect !== undefined) {
		let path = view.pathToSelect;
		delete view.pathToSelect;
		view.select(path, true);
	    }
	},

	onLoad: function(store, records, success) {
	    if (!success) {
		return;
	    }
	    let view = this.getView();
	    let root = view.getStore().getRoot();

	    records.sort((a, b) => a.id.localeCompare(b.id));

	    let list = root.findChild('id', 'datastores', false);
	    let getChildTextAt = i => list.getChildAt(i).data.text;
	    let existingChildren = {};
	    for (let i = 0, j = 0, length = records.length; i < length; i++) {
		let name = records[i].id;
		existingChildren[name] = true;

		while (name.localeCompare(getChildTextAt(j)) > 0 && (j+1) < list.childNodes.length) {
		    j++;
		}

		let [qtip, iconCls] = ['', 'fa fa-database'];
		const maintenance = records[i].data.maintenance;
		if (maintenance) {
		    const [type, message] = PBS.Utils.parseMaintenanceMode(maintenance);
		    qtip = `${type}${message ? ': ' + message : ''}`;
		    let mainenanceTypeCls = type === 'delete' ? 'destroying' : 'maintenance';
		    iconCls = `fa fa-database pmx-tree-icon-custom ${mainenanceTypeCls}`;
		}

		if (getChildTextAt(j).localeCompare(name) !== 0) {
		    list.insertChild(j, {
			text: name,
			qtip,
			path: `DataStore-${name}`,
			iconCls,
			leaf: true,
		    });
		} else {
		    let oldChild = list.getChildAt(j);
		    oldChild.set('qtip', qtip);
		    oldChild.set('iconCls', iconCls);
		}
	    }

	    // remove entries which are not existing anymore
	    let toRemove = [];
	    list.eachChild(child => {
		if (!existingChildren[child.data.text] && !child.data.virtualEntry) {
		    toRemove.push(child);
		}
	    });
	    toRemove.forEach(child => list.removeChild(child, true));

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
			destroy: () => me.rstore.reload(),
		    },
		}).show();
		return false;
	    }
	    return true;
	},
    },

    reloadTapeStore: function() {
	let me = this;
	me.tapeStore.load();
    },

    select: function(path, silent) {
	var me = this;
	if (me.rstore.isLoaded() && me.tapeStore.isLoaded()) {
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
});
