Ext.define('PBS.TapeManagement.BackupJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsTapeBackupJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    isAdd: true,

    subject: gettext('Tape Backup Job'),

    fieldDefaults: { labelWidth: 120 },

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/tape-backup-job';
	let id = initialConfig.id;

	me.isCreate = !id;
	me.url = id ? `${baseurl}/${id}` : baseurl;
	me.method = id ? 'PUT' : 'POST';
	me.autoLoad = !!id;
	me.scheduleValue = id ? null : 'daily';
	me.authid = id ? null : Proxmox.UserName;
	me.editDatastore = me.datastore === undefined && me.isCreate;
	return { };
    },

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    let me = this;

	    if (!values.id && me.up('pbsTapeBackupJobEdit').isCreate) {
		values.id = 's-' + Ext.data.identifier.Uuid.Global.generate().slice(0, 13);
	    }
	    return values;
	},
	column1: [
	    {
		xtype: 'pbsDataStoreSelector',
		fieldLabel: gettext('Local Datastore'),
		name: 'store',
	    },
	    {
		xtype: 'pbsMediaPoolSelector',
		fieldLabel: gettext('Media Pool'),
		name: 'pool',
	    },
	    {
		xtype: 'pbsDriveSelector',
		fieldLabel: gettext('Drive'),
		name: 'drive',
	    },
	],

	column2: [
	    {
		fieldLabel: gettext('Schedule'),
		xtype: 'pbsCalendarEvent',
		name: 'schedule',
		emptyText: gettext('none (disabled)'),
		cbind: {
		    deleteEmpty: '{!isCreate}',
		    value: '{scheduleValue}',
		},
	    },
	],

	columnB: [
	    {
		fieldLabel: gettext('Comment'),
		xtype: 'proxmoxtextfield',
		name: 'comment',
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	],
    },
});
