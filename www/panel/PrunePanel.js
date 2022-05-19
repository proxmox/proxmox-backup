Ext.define('PBS.panel.PruneInputPanel', {
    extend: 'Proxmox.panel.InputPanel',
    xtype: 'pbsPruneInputPanel',
    mixins: ['Proxmox.Mixin.CBind'],

    onlineHelp: 'maintenance_pruning',

    // show/hide dry-run field  FIXME: rename to canDryrun, this is confusing..
    dryrun: false,

    canRecurse: false, // show a recursive/max-depth field

    cbindData: function() {
	let me = this;
	me.isCreate = !!me.isCreate;
	return {
	    ns: me.ns ?? '',
	};
    },

    viewModel: {
	data: { canRecurse: false },
    },

    onGetValues: function(values) {
	let me = this;
	if (me.ns && me.ns !== '') {
	    values.ns = me.ns;
	}
	if (!values.recursive) {
	    values['max-depth'] = 0;
	}
	delete values.recursive;
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
	{
	    xtype: 'proxmoxcheckbox',
	    name: 'dry-run',
	    fieldLabel: gettext('Dry Run'),
	    cbind: {
		hidden: '{!dryrun}',
		disabled: '{!dryrun}',
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
	{
	    xtype: 'fieldcontainer',
	    layout: 'hbox',
	    fieldLabel: gettext('Recursive'),
	    cbind: {
		hidden: '{!canRecurse}',
		disabled: '{!canRecurse}',
	    },
	    items: [
		{
		    xtype: 'proxmoxcheckbox',
		    name: 'recursive',
		    uncheckedValue: false,
		    value: true,
		    bind: {
			value: '{canRecurse}',
		    },
		},
		{
		    xtype: 'pbsNamespaceMaxDepth',
		    name: 'max-depth',
		    padding: '0 0 0 5',
		    labelWidth: 75,
		    deleteEmpty: false,
		    bind: {
			disabled: '{!canRecurse}',
		    },
		    flex: 1,
		},
	    ],
	},
    ],
});
