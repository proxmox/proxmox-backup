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
	    xtype: 'pbsDataStoreContent',
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
	me.title = `${gettext("Data Store")}: ${me.datastore}`;
	me.callParent();
    },
});
