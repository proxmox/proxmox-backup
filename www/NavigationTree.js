Ext.define('PBS.store.NavigationStore', {
    extend: 'Ext.data.TreeStore',

    storeId: 'NavigationStore',

    root: {
	expanded: true,
	children: [
	    {
		text: gettext('Configuration'),
		iconCls: 'fa fa-gears',
		path: 'pbsSystemConfiguration',
		expanded: true,
		children: [
		    {
			text: gettext('Data Store'),
			iconCls: 'fa fa-envelope-o',
			path: 'pbsDataStoreConfiguration',
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
