Ext.define('PBS.Datastore.PruneAndGC', {
    extend: 'Ext.panel.Panel',
    alias: 'widget.pbsDatastorePruneAndGC',
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
    items: [
	{
	    xtype: 'pbsGCJobView',
	    itemId: 'gcjobs',
	    nodename: 'localhost',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
	{
	    xtype: 'pbsPruneJobView',
	    nodename: 'localhost',
	    itemId: 'prunejobs',
	    cbind: {
		datastore: '{datastore}',
	    },
	},
    ],
    initComponent: function() {
	let me = this;

	let subPanelIds = me.items.map(el => el.itemId);

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
