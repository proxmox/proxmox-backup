Ext.define('PBS.config.PruneAndGC', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsPruneAndGC',
    title: gettext('Prune & GC Jobs'),

    mixins: ['Proxmox.Mixin.CBind'],

    layout: {
	type: 'vbox',
	align: 'stretch',
	multi: true,
    },
    defaults: {
	collapsible: false,
	margin: '7 10 3 10',
    },
    scrollable: true,
    items: [
	{
	    xtype: 'pbsGCJobView',
	    itemId: 'gcjobs',
	    nodename: 'localhost',
	    cbind: {
		datastore: '{datastore}',
	    },
	    minHeight: 125, // shows at least one line of content
	},
	{
	    xtype: 'splitter',
	    performCollapse: false,
	},
	{
	    xtype: 'pbsPruneJobView',
	    nodename: 'localhost',
	    itemId: 'prunejobs',
	    cbind: {
		datastore: '{datastore}',
	    },
	    flex: 1,
	    minHeight: 160, // shows at least one line of content
	},
    ],
    initComponent: function() {
	let me = this;

	let subPanelIds = me.items.map(el => el.itemId).filter(id => !!id);

	me.callParent();

	for (const itemId of subPanelIds) {
	    let component = me.getComponent(itemId);
	    component.relayEvents(me, ['activate', 'deactivate', 'destroy']);
	}
    },

    cbindData: function(initalConfig) {
        let me = this;
        me.datastore = initalConfig.datastore ? initalConfig.datastore : undefined;
    },
});
