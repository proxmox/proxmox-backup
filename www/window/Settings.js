Ext.define('PBS.window.Settings', {
    extend: 'Ext.window.Window',

    width: '800px',
    title: gettext('My Settings'),
    iconCls: 'fa fa-gear',
    modal: true,
    bodyPadding: 10,
    resizable: false,

    buttons: [
	'->',
	{
	    text: gettext('Close'),
	    handler: function() {
		this.up('window').close();
	    },
	},
    ],

    layout: 'hbox',

    controller: {
	xclass: 'Ext.app.ViewController',

	init: function(view) {
	    let me = this;
	    let sp = Ext.state.Manager.getProvider();

	    let username = sp.get('login-username') || Proxmox.Utils.noneText;
	    me.lookupReference('savedUserName').setValue(Ext.String.htmlEncode(username));

	    let summarycolumns = sp.get('summarycolumns', 'auto');
	    me.lookup('summarycolumns').setValue(summarycolumns);

	    let settings = ['fontSize', 'fontFamily', 'letterSpacing', 'lineHeight'];
	    settings.forEach(function(setting) {
		let val = localStorage.getItem('pve-xterm-' + setting);
		if (val !== undefined && val !== null) {
		    let field = me.lookup(setting);
		    field.setValue(val);
		    field.resetOriginalValue();
		}
	    });
	},

	set_button_status: function() {
	    let me = this;

	    let form = me.lookup('xtermform');
	    let valid = form.isValid();
	    let dirty = form.isDirty();

	    let hasvalues = false;
	    let values = form.getValues();
	    Ext.Object.eachValue(values, function(value) {
		if (value) {
		    hasvalues = true;
		    return false;
		}
		return true;
	    });

	    me.lookup('xtermsave').setDisabled(!dirty || !valid);
	    me.lookup('xtermreset').setDisabled(!hasvalues);
	},

	control: {
	    '#xtermjs form': {
		dirtychange: 'set_button_status',
		validitychange: 'set_button_status',
	    },
	    '#xtermjs button': {
		click: function(button) {
		    let me = this;
		    let settings = ['fontSize', 'fontFamily', 'letterSpacing', 'lineHeight'];
		    settings.forEach(function(setting) {
			let field = me.lookup(setting);
			if (button.reference === 'xtermsave') {
			    let value = field.getValue();
			    if (value) {
				localStorage.setItem('pve-xterm-' + setting, value);
			    } else {
				localStorage.removeItem('pve-xterm-' + setting);
			    }
			} else if (button.reference === 'xtermreset') {
			    field.setValue(undefined);
			    localStorage.removeItem('pve-xterm-' + setting);
			}
			field.resetOriginalValue();
		    });
		    me.set_button_status();
		},
	    },
	    'button[name=reset]': {
		click: function() {
		    let blacklist = ['login-username'];
		    let sp = Ext.state.Manager.getProvider();
		    for (const state of Object.values(sp.state)) {
			if (blacklist.indexOf(state) !== -1) {
			    continue;
			}

			sp.clear(state);
		    }

		    window.location.reload();
		},
	    },
	    'button[name=clear-username]': {
		click: function() {
		    let me = this;
		    let usernamefield = me.lookupReference('savedUserName');
		    let sp = Ext.state.Manager.getProvider();

		    usernamefield.setValue(Proxmox.Utils.noneText);
		    sp.clear('login-username');
		},
	    },
	    'field[reference=summarycolumns]': {
		change: function(el, newValue) {
		    var sp = Ext.state.Manager.getProvider();
		    sp.set('summarycolumns', newValue);
		},
	    },
	},
    },

    items: [{
	xtype: 'fieldset',
	flex: 1,
	title: gettext('Webinterface Settings'),
	margin: '5',
	layout: {
	    type: 'vbox',
	    align: 'left',
	},
	defaults: {
	    width: '100%',
	    margin: '0 0 10 0',
	},
	items: [
	    {
		xtype: 'container',
		layout: 'hbox',
		items: [
		    {
			xtype: 'displayfield',
			fieldLabel: gettext('Saved User Name') + ':',
			labelWidth: 150,
			stateId: 'login-username',
			reference: 'savedUserName',
			flex: 1,
			value: '',
		    },
		    {
			xtype: 'button',
			cls: 'x-btn-default-toolbar-small proxmox-inline-button',
			text: gettext('Reset'),
			name: 'clear-username',
		    },
		],
	    },
	    {
		xtype: 'box',
		autoEl: { tag: 'hr' },
	    },
	    {
		xtype: 'container',
		layout: 'hbox',
		items: [
		    {
			xtype: 'displayfield',
			fieldLabel: gettext('Layout') + ':',
			flex: 1,
		    },
		    {
			xtype: 'button',
			cls: 'x-btn-default-toolbar-small proxmox-inline-button',
			text: gettext('Reset'),
			tooltip: gettext('Reset all layout changes (for example, column widths)'),
			name: 'reset',
		    },
		],
	    },
	    {
		xtype: 'box',
		autoEl: { tag: 'hr' },
	    },
	    {
		xtype: 'proxmoxKVComboBox',
		fieldLabel: gettext('Summary/Dashboard columns') + ':',
		labelWidth: 150,
		stateId: 'summarycolumns',
		reference: 'summarycolumns',
		comboItems: [
		    ['auto', 'auto'],
		    ['1', '1'],
		    ['2', '2'],
		    ['3', '3'],
		],
	    },
	],
    },
    {
	xtype: 'container',
	layout: 'vbox',
	flex: 1,
	margin: '5',
	defaults: {
	    width: '100%',
	    // right margin ensures that the right border of the fieldsets
	    // is shown
	    margin: '0 2 10 0',
	},
	items: [
	    {
		xtype: 'fieldset',
		itemId: 'xtermjs',
		title: gettext('xterm.js Settings'),
		items: [{
		    xtype: 'form',
		    reference: 'xtermform',
		    border: false,
		    layout: {
			type: 'vbox',
			algin: 'left',
		    },
		    defaults: {
			width: '100%',
			margin: '0 0 10 0',
		    },
		    items: [
			{
			    xtype: 'textfield',
			    name: 'fontFamily',
			    reference: 'fontFamily',
			    emptyText: Proxmox.Utils.defaultText,
			    fieldLabel: gettext('Font-Family'),
			},
			{
			    xtype: 'proxmoxintegerfield',
			    emptyText: Proxmox.Utils.defaultText,
			    name: 'fontSize',
			    reference: 'fontSize',
			    minValue: 1,
			    fieldLabel: gettext('Font-Size'),
			},
			{
			    xtype: 'numberfield',
			    name: 'letterSpacing',
			    reference: 'letterSpacing',
			    emptyText: Proxmox.Utils.defaultText,
			    fieldLabel: gettext('Letter Spacing'),
			},
			{
			    xtype: 'numberfield',
			    name: 'lineHeight',
			    minValue: 0.1,
			    reference: 'lineHeight',
			    emptyText: Proxmox.Utils.defaultText,
			    fieldLabel: gettext('Line Height'),
			},
			{
			    xtype: 'container',
			    layout: {
				type: 'hbox',
				pack: 'end',
			    },
			    defaults: {
				margin: '0 0 0 5',
			    },
			    items: [
				{
				    xtype: 'button',
				    reference: 'xtermreset',
				    disabled: true,
				    text: gettext('Reset'),
				},
				{
				    xtype: 'button',
				    reference: 'xtermsave',
				    disabled: true,
				    text: gettext('Save'),
				},
			    ],
			},
		    ],
		}],
	    },
	],
    }],
});
