Ext.define('PBS.BackupGroupChangeOwner', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsBackupGroupChangeOwner',

    submitText: gettext("Change Owner"),
    subject: gettext("Change Owner"),

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
	    items: {
		xtype: 'inputpanel',
		onGetValues: function(values) {
		    values["backup-type"] = me.backup_type;
		    values["backup-id"] = me.backup_id;
		    return values;
		},

		column1: [
		    {
			xtype: 'pbsAuthidSelector',
			name: 'new-owner',
			value: me.owner,
			fieldLabel: gettext('Owner'),
			minLength: 8,
			allowBlank: false,
		    },
		],
	    },
	});

	me.callParent();
    },
});
