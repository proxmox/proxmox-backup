Ext.define('PBS.panel.PruneInputPanel', {
    extend: 'Proxmox.panel.InputPanel',
    xtype: 'pbsPruneInputPanel',

    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'maintenance_pruning',

    // show/hide dry-run field
    dryrun: false,

    cbindData: function() {
	let me = this;
	me.isCreate = !!me.isCreate;
	return {
	    ns: me.ns ?? '',
	};
    },

    onGetValues: function(values) {
	if (values.ns === '') {
	    delete values.ns;
	}
	return values;
    },

    column1: [
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-last',
	    fieldLabel: gettext('Keep Last'),
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-daily',
	    fieldLabel: gettext('Keep Daily'),
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-monthly',
	    fieldLabel: gettext('Keep Monthly'),
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
    ],
    column2: [
	{
	    xtype: 'pbsPruneKeepInput',
	    fieldLabel: gettext('Keep Hourly'),
	    name: 'keep-hourly',
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-weekly',
	    fieldLabel: gettext('Keep Weekly'),
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
	{
	    xtype: 'pbsPruneKeepInput',
	    name: 'keep-yearly',
	    fieldLabel: gettext('Keep Yearly'),
	    cbind: {
		deleteEmpty: '{!isCreate}',
	    },
	},
    ],

    columnB: [
	{
	    xtype: 'proxmoxcheckbox',
	    name: 'dry-run',
	    fieldLabel: gettext('Dry Run'),
	    cbind: {
		hidden: '{!dryrun}',
		disabled: '{!dryrun}',
	    },
	},
	{
	    xtype: 'proxmoxtextfield',
	    name: 'ns',
	    hidden: true,
	    cbind: {
		value: '{ns}',
	    },
	},
    ],

});
