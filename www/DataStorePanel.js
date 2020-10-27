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
	    iconCls: 'fa fa-book',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    xtype: 'pbsDataStoreContent',
	    itemId: 'content',
	    iconCls: 'fa fa-th',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    title: gettext('Prune & Garbage collection'),
	    xtype: 'pbsDataStorePruneAndGC',
	    itemId: 'prunegc',
	    iconCls: 'fa fa-trash-o',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    iconCls: 'fa fa-refresh',
	    itemId: 'syncjobs',
	    xtype: 'pbsSyncJobView',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    iconCls: 'fa fa-check-circle',
	    itemId: 'verifyjobs',
	    xtype: 'pbsVerifyJobView',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    itemId: 'acl',
	    xtype: 'pbsACLView',
	    iconCls: 'fa fa-unlock',
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
