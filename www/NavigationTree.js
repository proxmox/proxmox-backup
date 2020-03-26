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
		model: 'pbs-data-store-config'
	    });

	    view.rstore.on('load', this.onLoad, this);
	    view.on('destroy', view.rstore.stopUpdate);
	},

	onLoad: function(store, records, success) {
	    var view = this.getView();

	    let root = view.getStore().getRoot();

	    if (!root.findChild('path', 'pbsDataStoreConfig', false)) {
		root.appendChild({
		    text: gettext('Data Store'),
		    expanded: true,
		    iconCls: 'fa fa-archive',
		    path: 'pbsDataStoreConfig',
		    leaf: false
		});
	    }

	    var list = root.findChild('path', 'pbsDataStoreConfig', false);
	    var length = records.length;
	    var lookup_hash = {};
	    for (var i = 0; i < length; i++) {
		var name = records[i].id;
		lookup_hash[name] = true;
		if (!list.findChild('text', name, false)) {
		    list.appendChild({
			text: name,
			path: 'pbsDataStoreContent:' + name,
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
