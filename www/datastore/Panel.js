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

    stateId: 'pbs-datastore-panel',
    stateful: true,

    stateEvents: ['tabchange'],

    applyState: function(state) {
	let me = this;
	if (state.tab !== undefined && me.rendered) {
	    me.setActiveTab(state.tab);
	} else if (state.tab) {
	    // if we are not rendered yet, defer setting the activetab
	    setTimeout(() => me.setActiveTab(state.tab), 10);
	}
    },

    getState: function() {
	let me = this;
	return {
	    tab: me.getActiveTab().getItemId(),
	};
    },

    border: false,
    defaults: {
	border: false,
    },

    tools: [PBS.Utils.get_help_tool("datastore_intro")],

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
	    title: gettext('Prune & GC'),
	    xtype: 'pbsDatastorePruneAndGC',
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
	    xtype: 'pbsDatastoreOptionView',
	    itemId: 'options',
	    title: gettext('Options'),
	    iconCls: 'fa fa-cog',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    itemId: 'acl',
	    xtype: 'pbsACLView',
	    iconCls: 'fa fa-unlock',
	    cbind: {
		aclPath: '{aclPath}',
		datastore: '{datastore}',
	    },
	},
    ],

    initComponent: function() {
	let me = this;
	me.title = `${gettext("Datastore")}: ${me.datastore}`;
	// remove invalid activeTab settings
	if (me.activeTab && !me.items.some((item) => item.itemId === me.activeTab)) {
	    delete me.activeTab;
	}
	me.callParent();
    },
});
