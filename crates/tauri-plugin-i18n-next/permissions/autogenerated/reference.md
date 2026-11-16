## Default Permission

What the front end needs (DESIGN 8b.2): load the payload and pages, read and switch the locale. `report_missing` (dev only) is not included: add `i18n-next:allow-report-missing` to a dev capability.

#### This default permission set includes the following:

- `allow-load`
- `allow-load-page`
- `allow-get-locale`
- `allow-available-locales`
- `allow-pages`
- `allow-set-locale`

## Permission Table

<table>
<tr>
<th>Identifier</th>
<th>Description</th>
</tr>


<tr>
<td>

`i18n-next:allow-available-locales`

</td>
<td>

Enables the available_locales command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:deny-available-locales`

</td>
<td>

Denies the available_locales command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:allow-get-locale`

</td>
<td>

Enables the get_locale command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:deny-get-locale`

</td>
<td>

Denies the get_locale command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:allow-load`

</td>
<td>

Enables the load command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:deny-load`

</td>
<td>

Denies the load command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:allow-load-page`

</td>
<td>

Enables the load_page command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:deny-load-page`

</td>
<td>

Denies the load_page command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:allow-pages`

</td>
<td>

Enables the pages command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:deny-pages`

</td>
<td>

Denies the pages command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:allow-report-missing`

</td>
<td>

Enables the report_missing command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:deny-report-missing`

</td>
<td>

Denies the report_missing command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:allow-set-locale`

</td>
<td>

Enables the set_locale command without any pre-configured scope.

</td>
</tr>

<tr>
<td>

`i18n-next:deny-set-locale`

</td>
<td>

Denies the set_locale command without any pre-configured scope.

</td>
</tr>
</table>
