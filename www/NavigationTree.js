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
		leaf: true
	    },
	    {
		text: gettext('Configuration'),
		iconCls: 'fa fa-gears',
		path: 'pbsSystemConfiguration',
		expanded: true,
		children: [
		    {
			text: gettext('User Management'),
			iconCls: 'fa fa-user',
			path: 'pbsUserView',
			leaf: true
		    },
		    {
			text: gettext('Permissions'),
			iconCls: 'fa fa-unlock',
			path: 'pbsACLView',
			leaf: true
		    },
		    {
			text: gettext('Remotes'),
			iconCls: 'fa fa-server',
			path: 'pbsRemoteView',
			leaf: true,
		    },
		    {
			text: gettext('Data Store'),
			iconCls: 'fa fa-archive',
			path: 'pbsDataStoreConfig',
			leaf: true
		    },
		    {
			text: gettext('Subscription'),
			iconCls: 'fa fa-support',
			path: 'pbsSubscription',
			leaf: true
		    }
		]
	    },
	    {
		text: gettext('Administration'),
		iconCls: 'fa fa-wrench',
		path: 'pbsServerAdministration',
		leaf: true
	    }
	]
    }
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
		model: 'pbs-datastore-list'
	    });

	    view.rstore.on('load', this.onLoad, this);
	    view.on('destroy', view.rstore.stopUpdate);
	},

	onLoad: function(store, records, success) {
	    var view = this.getView();

	    let root = view.getStore().getRoot();

	    if (!root.findChild('path', 'pbsDataStoreStatus', false)) {
		root.appendChild({
		    text: gettext('Data Store'),
		    expanded: true,
		    iconCls: 'fa fa-archive',
		    path: 'pbsDataStoreStatus',
		    leaf: false
		});
	    }

	    // FIXME: newly added always get appended to the end..
	    records.sort((a, b) => {
		if (a.id > b.id) return  1;
		if (a.id < b.id) return -1;
		return 0;
	    });

	    var list = root.findChild('path', 'pbsDataStoreStatus', false);
	    var length = records.length;
	    var lookup_hash = {};
	    for (var i = 0; i < length; i++) {
		var name = records[i].id;
		lookup_hash[name] = true;
		if (!list.findChild('text', name, false)) {
		    list.appendChild({
			text: name,
			path: `DataStore-${name}`,
			iconCls: 'fa fa-database',
			leaf: true
		    });
		}
	    }

	    var erase_list = [];
	    list.eachChild(function(node) {
		var name = node.data.text;
		if (!lookup_hash[name]) {
		    erase_list.push(node);
		}
	    });

	    Ext.Array.forEach(erase_list, function(node) { node.erase(); });

	}
    },

    select: function(path) {
	var me = this;
	var item = me.getStore().findRecord('path', path, 0, false, true, true);
	me.setSelection(item);
    },

    animation: false,
    expanderOnly: true,
    expanderFirst: false,
    store: 'NavigationStore',
    ui: 'nav'
});
