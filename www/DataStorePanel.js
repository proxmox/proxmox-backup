Ext.define('PBS.DataStorePanel', {
    extend: 'Ext.tab.Panel',
    alias: 'widget.pbsDataStorePanel',
    mixins: ['Proxmox.Mixin.CBind'],

    cbindData: function(initalConfig) {
	let me = this;
	return {
	    aclPath: `/datastore/${me.datastore}`,
	};
    },

    border: false,
    defaults: {
	border: false,
    },

    items: [
	{
	    xtype: 'pbsDataStoreSummary',
	    title: gettext('Summary'),
	    itemId: 'summary',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    xtype: 'pbsDataStoreContent',
	    itemId: 'content',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    title: gettext('Prune & Garbage collection'),
	    xtype: 'pbsDataStorePruneAndGC',
	    itemId: 'prunegc',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    itemId: 'acl',
	    xtype: 'pbsACLView',
	    aclExact: true,
	    cbind: {
		aclPath: '{aclPath}',
	    },
	},
    ],

    initComponent: function() {
	let me = this;
	me.title = `${gettext("Datastore")}: ${me.datastore}`;
	me.callParent();
    },
});
