Ext.define('PBS.BackupGroupChangeOwner', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsBackupGroupChangeOwner',

    onlineHelp: 'changing-backup-owner',

    submitText: gettext("Change Owner"),
    width: 350,

    initComponent: function() {
	let me = this;

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
	    url: `/api2/extjs/admin/datastore/${me.datastore}/change-owner`,
	    method: 'POST',
	    subject: gettext("Change Owner") + ` - ${me.backup_type}/${me.backup_id}`,
	    items: {
		xtype: 'inputpanel',
		onGetValues: function(values) {
		    values["backup-type"] = me.backup_type;
		    values["backup-id"] = me.backup_id;
		    if (me.ns && me.ns !== '') {
			values.ns = me.ns;
		    }
		    return values;
		},

		items: [
		    {
			xtype: 'pbsAuthidSelector',
			name: 'new-owner',
			value: me.owner,
			fieldLabel: gettext('New Owner'),
			minLength: 8,
			allowBlank: false,
		    },
		],
	    },
	});

	me.callParent();
    },
});
