Ext.define('PBS.window.SyncJobEdit', {
    extend: 'Proxmox.window.Edit',
    alias: 'widget.pbsSyncJobEdit',
    mixins: ['Proxmox.Mixin.CBind'],

    userid: undefined,

    onlineHelp: 'syncjobs',

    isAdd: true,

    subject: gettext('SyncJob'),

    fieldDefaults: { labelWidth: 120 },
    defaultFocus: 'proxmoxtextfield[name=comment]',

    cbindData: function(initialConfig) {
	let me = this;

	let baseurl = '/api2/extjs/config/sync';
	let id = initialConfig.id;

	me.isCreate = !id;
	me.url = id ? `${baseurl}/${id}` : baseurl;
	me.method = id ? 'PUT' : 'POST';
	me.autoLoad = !!id;
	me.scheduleValue = id ? null : 'hourly';
	return { };
    },

    items: {
	xtype: 'inputpanel',
	onGetValues: function(values) {
	    let me = this;

	    if (!values.id && me.up('pbsSyncJobEdit').isCreate) {
		values.id = 'auto-' + Ext.data.identifier.Uuid.Global.generate().slice(0, 23);
	    }
	    return values;
	},
	column1: [
	    {
		xtype: 'displayfield',
		name: 'store',
		fieldLabel: gettext('Local Datastore'),
		allowBlank: false,
		submitValue: true,
		cbind: {
		    value: '{datastore}',
		},
	    },
	    {
		fieldLabel: gettext('Local Owner'),
		xtype: 'pbsUserSelector',
		name: 'owner',
		allowBlank: true,
		value: null,
		emptyText: 'backup@pam',
		skipEmptyText: true,
		cbind: {
		    deleteEmpty: '{!isCreate}',
		},
	    },
	    {
		fieldLabel: gettext('Remove vanished'),
		xtype: 'proxmoxcheckbox',
		name: 'remove-vanished',
		uncheckedValue: false,
		value: false,
	    },
	],

	column2: [
	    {
		fieldLabel: gettext('Source Remote'),
		xtype: 'pbsRemoteSelector',
		allowBlank: false,
		name: 'remote',
	    },
	    {
		fieldLabel: gettext('Source Datastore'),
		xtype: 'proxmoxtextfield',
		allowBlank: false,
		name: 'remote-store',
	    },
	    {
		fieldLabel: gettext('Sync Schedule'),
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
