Ext.define('PBS.DataStorePruneInputPanel', {
    extend: 'Proxmox.panel.InputPanel',
    alias: 'widget.pbsDataStorePruneInputPanel',

    items: [
	{
	    xtype: 'proxmoxintegerfield',
	    name: 'keep-last',
	    allowBlank: true,
	    fieldLabel: gettext('keep-last'),
	    minValue: 1,
	},
	{
	    xtype: 'proxmoxintegerfield',
	    name: 'keep-hourly',
	    allowBlank: true,
	    fieldLabel: gettext('keep-hourly'),
	    minValue: 1,
	},
	{
	    xtype: 'proxmoxintegerfield',
	    name: 'keep-daily',
	    allowBlank: true,
	    fieldLabel: gettext('keep-daily'),
	    minValue: 1,
	},
	{
	    xtype: 'proxmoxintegerfield',
	    name: 'keep-weekly',
	    allowBlank: true,
	    fieldLabel: gettext('keep-weekly'),
	    minValue: 1,
	},
	{
	    xtype: 'proxmoxintegerfield',
	    name: 'keep-monthly',
	    allowBlank: true,
	    fieldLabel: gettext('keep-monthly'),
	    minValue: 1,
	},
	{
	    xtype: 'proxmoxintegerfield',
	    name: 'keep-yearly',
	    allowBlank: true,
	    fieldLabel: gettext('keep-yearly'),
	    minValue: 1,
	}
	// fixme: howto handle dry-run?
	//{
	//    xtype: 'proxmoxcheckbox',
	//    name: 'dry-run',
	//    fieldLabel: gettext('dry-run'),
	//}
     ],

});

Ext.define('PBS.DataStorePrune', {
    extend: 'Proxmox.window.Edit',

    method: 'POST',
    submitText: "Prune",

    isCreate: true,

    initComponent : function() {
        var me = this;

	if (!me.datastore) {
	    throw "no datastore specified";
	}
	if (!me.backup_type) {
	    throw "no backup_type specified";
	}
	if (!me.backup_id) {
	    throw "no backup_id specified";
	}

	Ext.apply(me, {
	    url: '/api2/extjs/admin/datastore/' + me.datastore + "/prune",
	    title: "Prune Datastore '" + me.datastore + "'",
	    items: [{
		xtype: 'pbsDataStorePruneInputPanel',
		onGetValues: function(values) {
		    values["backup-type"] = me.backup_type;
		    values["backup-id"] = me.backup_id;
		    return values;
		}
	    }]
	});

	me.callParent();
    }
});
